# TC-INTERRUPT-014
# PASSIVE P8: the hub's DB WRITE FAILS while settling a turn. A real, exclusive writer lock is
# held on the LIVE `hub.sqlite` at the settle instant (a real store failure - the same
# `database is locked` a second writer hits), so BOTH the durable terminal intent and the
# terminal write fail. The hub must NOT lie: it must never report a terminal the store does
# not hold (never a false `completed`), the API and the store must AGREE (both un-settled),
# and the next BOOT must reconcile the open turn honestly (`failed`) - never `completed`.
#
# FACT:    a DB write failure while settling leaves the turn un-settled and honest, and a boot settles it `failed`
# SOURCE:  TASK-048 F4/S3 (settle_turn records terminal_intent, retries, never publishes on failure); ARCHITECTURE s21
# EXPOSES: a terminal announced the store does not hold, or a turn stuck `completed` after a failed write
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import json
import os
import sqlite3
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn            # noqa: E402
from tally import Tally, combo, kind_of                                # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/db-write-fails-settling")
combo(hub_sha(), kind_of(PI))

HOME = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".test-home"))
DBF = os.path.join(HOME, "hub.sqlite")


def store_turn(tid):
    """The turn straight out of the live store - the store's own answer, not the API's."""
    conn = sqlite3.connect(DBF)
    conn.row_factory = sqlite3.Row
    try:
        for r in conn.execute("SELECT id, state, ended, terminal_intent FROM turns WHERE id=?1", (tid,)):
            return dict(r)
        return {}
    finally:
        conn.close()


def api_turn(hub, sid, tid):
    body = json.loads(hub.get(f"/v1/sessions/{sid}/turns")["text"])
    return next((x for x in body["turns"] if x["id"] == tid), None)


def acquire_exclusive(dbf):
    """Really hold the store's write lock, like a second writer mid-transaction. Bounded retry:
    the hub may be mid-write when we arrive."""
    conn = sqlite3.connect(dbf, timeout=0)
    conn.isolation_level = None
    for _ in range(600):
        try:
            conn.execute("BEGIN EXCLUSIVE")
            return conn
        except sqlite3.OperationalError:
            time.sleep(0.05)
    conn.close()
    return None


hub = Hub(plugins_src=PI, data_dir=HOME)
lk = None
sid = tid = None
sid2 = None
ok_boot = None
try:
    if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
        raise SystemExit(0)
    hub.start()
    PROV = register_provider(hub)
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="db-s")
    sid = r["json"]["session"]["id"]
    hub.wait_status(sid, "active")

    # A long turn, so we hold the lock WHILE it runs and the settle lands inside the lock.
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Count slowly from 1 to 3000, one number per line."}],
                   "idempotencyKey": "db-t"}, key="db-t")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval"}, tries=160)
    t.check(row.get("state") in ("running", "awaiting_approval"), "the turn runs before the store fails",
            f"state={row.get('state')}")

    # HOLD THE WRITE LOCK on the live store for well past the settle.
    lk = acquire_exclusive(DBF)
    if not t.require(lk is not None, "the test can hold the live store's write lock", "could not acquire the lock"):
        raise SystemExit(0)

    # Through the whole settlement attempt the API must stay HONEST: never a terminal the store
    # does not hold. Watch both sides agree.
    deadline = time.time() + 18
    lie = None
    while time.time() < deadline:
        a = api_turn(hub, sid, tid) or {}
        d = store_turn(tid)
        if a.get("state") == "ended" and d.get("ended") is None:
            lie = ("api-terminal-without-store-terminal", a.get("state"), a.get("ended"), d.get("ended"))
        if a.get("ended") == "completed" and d.get("ended") != "completed":
            lie = ("completed-lie", a.get("ended"), d.get("ended"))
        time.sleep(0.5)

    t.check(lie is None, "the hub never reports a terminal the store does not hold (API and store agree)",
            f"lie={lie}")
    held = store_turn(tid)
    t.check(held.get("ended") != "completed", "a failed settle never leaves a `completed` in the store",
            f"store ended={held.get('ended')}")

    # Release the lock; the settle already exhausted its retries, so the turn is left OPEN and
    # honest (not a lie, just un-settled).
    lk.execute("COMMIT"); lk.close(); lk = None
    time.sleep(1.5)
    after = api_turn(hub, sid, tid) or {}
    t.check(after.get("ended") != "completed", "after the lock lifts the turn is not falsely completed",
            f"state={after.get('state')} ended={after.get('ended')}")

    t.check(hub.child.poll() is None, "the hub survives the store write failure")
finally:
    if lk is not None:
        try:
            lk.execute("COMMIT")
        except Exception:
            pass
        lk.close()

# BOOT on the same home: reconciliation must settle the open turn honestly - `failed`, never a
# fabricated `completed`. The store is the source of truth and the API must match it.
hub2 = Hub(data_dir=HOME)
try:
    hub2.start()
    time.sleep(2.5)
    db = store_turn(tid)
    t.check(db.get("ended") == "failed",
            "boot reconciliation settles the un-settled turn `failed` (never `completed`)",
            f"store ended={db.get('ended')} state={db.get('state')}")
    api = api_turn(hub2, sid, tid) if sid else None
    if api is not None:
        t.check(api.get("ended") == db.get("ended"),
                "after boot, the API and the store agree on the terminal",
                f"api={api.get('ended')} store={db.get('ended')}")
finally:
    hub2.stop()
    ok = t.done()
sys.exit(0 if ok else 1)
