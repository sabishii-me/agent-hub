# agent-hub-events

The hub's event bus (`ARCHITECTURE` §11, task T4). WHATWG framing says *how* an event
is written; this crate supplies the *delivery* semantics:

- every change gets a **monotonically increasing id** (never reused);
- a **bounded replay buffer** allows `Last-Event-ID` catch-up;
- an id that has **fallen out** of the window, or a **future** id, yields `Resync`
  (the client re-reads the resource) - never a silent gap;
- each subscriber has a **bounded** queue; a slow subscriber is dropped for resync
  instead of blocking the publisher (`ADR-0009`).
