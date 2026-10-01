//! The hub's event stream (`ARCHITECTURE` §11, task T4): a monotonic sequence
//! with a bounded replay window, and subscribers that converge under drop,
//! overflow and restart.
//!
//! WHATWG framing (`id:` / `Last-Event-ID`) only says *how* an event is framed;
//! this module supplies the *delivery* semantics the contract requires:
//!
//! - every change gets a **monotonically increasing id** (never reused);
//! - a **bounded replay buffer** keeps the last `replay_capacity` changes, so a
//!   client that reconnects with `Last-Event-ID` can be caught up;
//! - if the requested id has **fallen out** of the window, the subscriber is
//!   told to **re-read the resource** instead (an event says *when* to re-read;
//!   the resource says what is true);
//! - each subscriber gets a **bounded** channel; a slow subscriber that
//!   overflows is marked as needing a re-read rather than blocking the
//!   publisher (`ADR-0009`: no request path blocks).

use serde::Serialize;

/// A delivered change: a sequence id, the event name, and a JSON payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub id: u64,
    pub name: String,
    pub data: serde_json::Value,
}

/// What a subscriber must do after connecting with `Last-Event-ID`.
#[derive(Debug, Clone, PartialEq)]
pub enum CatchUp {
    /// The id is in the window: replay everything after it.
    Replay(Vec<Event>),
    /// No id was given: start from now.
    Fresh,
    /// The id is unknown or has fallen out of the window: the subscriber cannot
    /// be told what it missed, so it must re-read the resources it cares about.
    Resync,
}

#[derive(Debug)]
struct Inner {
    next_id: u64,
    replay: std::collections::VecDeque<Event>,
    replay_capacity: usize,
    subscribers: Vec<tokio::sync::mpsc::Sender<Event>>,
}

/// The event bus: publish changes, subscribe with bounded backpressure.
#[derive(Clone)]
pub struct Bus {
    inner: std::sync::Arc<std::sync::Mutex<Inner>>,
    subscriber_capacity: usize,
}

/// A subscribed receiver plus the catch-up instruction.
pub struct Subscription {
    pub catch_up: CatchUp,
    pub receiver: tokio::sync::mpsc::Receiver<Event>,
}

impl Bus {
    /// `replay_capacity` is how many past changes are retained for
    /// `Last-Event-ID`; `subscriber_capacity` is each subscriber's queue depth.
    pub fn new(replay_capacity: usize, subscriber_capacity: usize) -> Self {
        Bus {
            inner: std::sync::Arc::new(std::sync::Mutex::new(Inner {
                next_id: 1,
                replay: std::collections::VecDeque::with_capacity(replay_capacity),
                replay_capacity,
                subscribers: Vec::new(),
            })),
            subscriber_capacity,
        }
    }

    /// Publish a change. Returns its sequence id.
    ///
    /// A subscriber whose queue is full is **dropped** (it will re-read); the
    /// publisher never blocks on a slow subscriber.
    pub fn publish(&self, name: impl Into<String>, data: serde_json::Value) -> u64 {
        let name = name.into();
        let mut inner = self.inner.lock().expect("bus mutex");
        let id = inner.next_id;
        inner.next_id += 1;

        let event = Event { id, name, data };
        inner.replay.push_back(event.clone());
        if inner.replay.len() > inner.replay_capacity {
            inner.replay.pop_front();
        }

        inner.subscribers.retain(|tx| match tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Slow subscriber: drop it. Its next connect gets Resync.
                tracing::warn!(id, "subscriber overflowed; dropping for resync");
                false
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        });

        id
    }

    /// Subscribe, optionally catching up from `last_event_id`.
    pub fn subscribe(&self, last_event_id: Option<u64>) -> Subscription {
        let (tx, rx) = tokio::sync::mpsc::channel(self.subscriber_capacity);
        let mut inner = self.inner.lock().expect("bus mutex");

        let catch_up = match last_event_id {
            None => CatchUp::Fresh,
            Some(want) => {
                let oldest = inner.replay.front().map(|e| e.id);
                if let Some(oldest) = oldest {
                    if want + 1 < oldest {
                        // Asked for something already evicted.
                        CatchUp::Resync
                    } else {
                        let replay: Vec<Event> = inner
                            .replay
                            .iter()
                            .filter(|e| e.id > want)
                            .cloned()
                            .collect();
                        CatchUp::Replay(replay)
                    }
                } else if want >= inner.next_id {
                    // Nothing has been published; an id beyond us is unknown.
                    CatchUp::Resync
                } else {
                    CatchUp::Fresh
                }
            }
        };

        inner.subscribers.push(tx);
        Subscription { catch_up, receiver: rx }
    }

    /// The most recent sequence id handed out (0 before anything is published).
    pub fn current_id(&self) -> u64 {
        self.inner.lock().expect("bus mutex").next_id - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn fresh_subscriber_starts_from_now() {
        let bus = Bus::new(8, 8);
        bus.publish("a", json!(1));
        let mut sub = bus.subscribe(None);
        assert_eq!(sub.catch_up, CatchUp::Fresh);
        bus.publish("b", json!(2));
        let e = sub.receiver.recv().await.unwrap();
        assert_eq!((e.id, e.name.as_str()), (2, "b"));
    }

    #[tokio::test]
    async fn last_event_id_replays_within_window() {
        let bus = Bus::new(8, 8);
        bus.publish("a", json!(1));
        bus.publish("b", json!(2));
        let sub = bus.subscribe(Some(1));
        match sub.catch_up {
            CatchUp::Replay(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].id, 2);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn evicted_id_forces_resync() {
        let bus = Bus::new(2, 8);
        for i in 0..5 {
            bus.publish("x", json!(i));
        }
        // Window holds ids 4,5; asking for id 1 falls out.
        let sub = bus.subscribe(Some(1));
        assert_eq!(sub.catch_up, CatchUp::Resync);
    }

    #[tokio::test]
    async fn unknown_future_id_forces_resync() {
        let bus = Bus::new(8, 8);
        let sub = bus.subscribe(Some(99));
        assert_eq!(sub.catch_up, CatchUp::Resync);
    }

    #[tokio::test]
    async fn slow_subscriber_is_dropped_not_blocked() {
        let bus = Bus::new(64, 1);
        let _sub = bus.subscribe(None);
        // Publish more than the subscriber queue holds; the publisher must not
        // block, and the subscriber is dropped.
        for i in 0..10 {
            bus.publish("x", json!(i));
        }
        assert_eq!(bus.current_id(), 10);
    }

    #[tokio::test]
    async fn ids_are_monotonic_and_never_reused() {
        let bus = Bus::new(4, 4);
        let a = bus.publish("a", json!(1));
        let b = bus.publish("b", json!(2));
        assert_eq!((a, b), (1, 2));
        assert_eq!(bus.current_id(), 2);
    }
}
