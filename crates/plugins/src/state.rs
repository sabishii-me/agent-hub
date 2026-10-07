//! The hub's **one verdict** on a plugin (`ARCHITECTURE`, the contract's `state`).
//!
//! The old hub kept a set of in-flight operations and ranked them, so a plugin
//! could never be reported as `removing` and `preparing` at once: there is one
//! `state`, and clients derive nothing. This mirrors that: [`Ops`] holds the
//! concurrent operations, [`Ops::verdict`] collapses them to one state, and a
//! `failed` recorded on disk outlives any single operation.

/// An operation a plugin is currently undergoing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Installing,
    Removing,
    Preparing,
    Failed,
}

impl Op {
    fn rank(self) -> u8 {
        // The old hub's ranking: removing > installing > preparing > failed.
        match self {
            Op::Removing => 3,
            Op::Installing => 2,
            Op::Preparing => 1,
            Op::Failed => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Installing => "installing",
            Op::Removing => "removing",
            Op::Preparing => "preparing",
            Op::Failed => "failed",
        }
    }
}

/// The set of operations currently running on one plugin.
#[derive(Debug, Clone, Default)]
pub struct Ops(Vec<Op>);

impl Ops {
    pub fn enter(&mut self, op: Op) {
        if !self.0.contains(&op) {
            self.0.push(op);
        }
    }

    pub fn leave(&mut self, op: Op) {
        self.0.retain(|o| *o != op);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The single verdict: the highest-ranked operation, or `None` when idle.
    pub fn verdict(&self) -> Option<Op> {
        self.0.iter().copied().max_by_key(|o| o.rank())
    }

    /// The `state` string for the contract.
    pub fn state_str(&self, has_dir: bool) -> &'static str {
        match self.verdict() {
            Some(op) => op.as_str(),
            None => {
                if has_dir {
                    "ready"
                } else {
                    "absent"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_verdict_never_two_states() {
        let mut ops = Ops::default();
        ops.enter(Op::Preparing);
        ops.enter(Op::Removing);
        // Even with both in flight, the verdict is one: removing wins.
        assert_eq!(ops.state_str(true), "removing");
        ops.leave(Op::Removing);
        assert_eq!(ops.state_str(true), "preparing");
        ops.leave(Op::Preparing);
        assert_eq!(ops.state_str(true), "ready");
        assert_eq!(ops.state_str(false), "absent");
    }

    #[test]
    fn failed_is_lowest_but_outlives() {
        let mut ops = Ops::default();
        ops.enter(Op::Failed);
        assert_eq!(ops.state_str(true), "failed");
        // A newer operation supersedes the failed verdict but does not erase it
        // while it is recorded.
        ops.enter(Op::Installing);
        assert_eq!(ops.state_str(true), "installing");
        ops.leave(Op::Installing);
        assert_eq!(ops.state_str(true), "failed");
    }
}
