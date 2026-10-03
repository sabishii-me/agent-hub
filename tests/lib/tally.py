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

    def check(self, ok, what, detail=""):
        if ok:
            self.passed += 1
            print(f"  ok   {what}")
        else:
            self.failed += 1
            print(f"  FAIL {what}" + (f" - {detail}" if detail else ""))
        return bool(ok)

    def skip(self, what, why):
        self.skipped += 1
        print(f"  skip {what} - {why}")

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
