"""Result tally + the version combo every result prints.

No fake: a check reports what actually happened. A SKIP is not a pass - it is a recorded
inability to run, so a suite can be honest about what it did NOT prove.
"""
import platform
import subprocess


class Tally:
    def __init__(self, name):
        self.name = name
        self.passed = 0
        self.failed = 0
        self.skipped = 0
        self.blocked = 0

    def check(self, ok, what, detail=""):
        if ok:
            self.passed += 1
            print(f"  ok   {what}")
        else:
            self.failed += 1
            print(f"  FAIL {what}" + (f" - {detail}" if detail else ""))
        return bool(ok)

    def blocked_check(self, what, why):
        """REMOVED as a state. A missing precondition is a FAILURE with a reason, never a
        separate 'BLOCKED' outcome: any BLOCK is a T0 (the environment or the test is wrong).
        Kept as a name so callers fail loudly instead of silently skipping."""
        self.failed += 1
        print(f"  FAIL {what} - {why}")
        return False

    def require(self, condition, what, missing):
        """A PRECONDITION of the test. A missing real dependency is a FAILURE, not a
        skip: the test exists to prove a capability against the real thing, and a
        capability that could not be exercised is not proven. Never a fake green."""
        if condition:
            return True
        self.failed += 1
        print(f"  FAIL {what} - {missing}")
        return False

    def done(self):
        total = self.passed + self.failed
        print(f"  [{self.name}] {self.passed}/{total} ok, {self.failed} failed, {self.skipped} skipped")
        return self.failed == 0


def kind_of(pdir):
    """A label for the real artifact under test, read from the plugin's manifest."""
    try:
        import json, pathlib
        m = json.loads((pathlib.Path(pdir) / "manifest.json").read_text(encoding="utf-8"))
        rt = m.get("runtime") or {}
        return f"plugin={m.get('id')}@{m.get('version')} runtime={rt.get('package')}@{rt.get('version')}"
    except Exception:
        return f"plugin={pdir}"


def combo(hub_sha, extra=""):
    """The version combo a result is attributable to. Printed once per file."""
    try:
        osname = platform.platform()
    except Exception:
        osname = "?"
    line = f"combo: hub={hub_sha} platform={osname}"
    if extra:
        line += f" {extra}"
    print(line)


class Blocked(Exception):
    """REMOVED as an outcome. A test that cannot exercise a real precondition now FAILS with the
    reason (exit 1), because a BLOCK is a T0: a missing precondition means the environment or the
    test is wrong, not that the result is 'unverified'. Kept so old callers fail loudly."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def blocked(reason):
    """A BLOCK is a T0. Print it as a FAILURE and exit NON-ZERO (1), never the old sentinel 3."""
    print(f"FAIL: {reason}")
    raise SystemExit(1)
