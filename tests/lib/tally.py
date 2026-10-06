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
        """A capability that CANNOT be exercised because a real PRECONDITION is missing (e.g.
        the official registry is not published). NOT a pass and NOT a fail: it is recorded as
        unverified, so no result claims it worked. Explicitly distinct from a failure (the code
        may be right) and from a pass (nothing was proven)."""
        self.blocked += 1
        print(f"  BLOCKED {what} - {why}")
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
        print(f"  [{self.name}] {self.passed}/{total} ok, {self.failed} failed, "
              f"{self.skipped} skipped, {self.blocked} BLOCKED")
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
    """Raised when a test CANNOT run because a real precondition is missing (e.g. the official
    registry is unpublished). It is NOT a failure (the code may be right) and NOT a pass (nothing
    was proven). `run.py` counts it separately. A test raises this - never fabricates the
    precondition to keep going (owner direction: no mock registry, no test-side stand-in)."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def blocked(reason):
    """Print a BLOCKED line and exit with the BLOCKED sentinel code (3)."""
    print(f"BLOCKED: {reason}")
    raise SystemExit(3)
