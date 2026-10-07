# 20261005-070000 — concurrent installs of the same plugin return 500 and trample one shared staging dir

Recorded 2026-10-05. Found by `tests/plugins/concurrent-install-and-remove.py`. Owner: **HUB**.

## Observed (real)

6 concurrent `POST /v1/plugins` for the SAME plugin (local path source):

```
500 {"error":"internal_error","detail":"io: Access is denied. (os error 5)"}   x5
202 {"pluginId":"pi","state":"installing"}                                      x1
```

## Two defects

1. **The staging dir is SHARED and unguarded.** `begin_install` uses ONE staging dir
   (`<root>/.stage/install`), does `remove_dir_all` then `create_dir_all`, and copies the tree
   into it - all BEFORE the plugin id is known. `begin_install`'s own comment says "one at a
   time per root", but **nothing enforces it**: concurrent installs stomp each other's staging
   and `copy_dir`/`remove_dir_all` hit Windows `os error 5`.
2. **The busy check is too late AND the status is wrong.** The `current_op(id) -> Busy` guard
   runs AFTER staging and only for a KNOWN id, so a concurrent install of the same id slips
   through to the 500. Per the owner: installing the SAME plugin concurrently must be REFUSED
   with a clear **409 conflict**, never a 500 internal_error.

## Fix direction (HUB)

Serialize install/remove at the service (one op at a time per root - what the staging comment
promises), so the shared staging and the ops map are touched one at a time; a second concurrent
install of the same id is refused **409 conflict**. Do NOT give each request its own staging as
the only fix - the op set must still serialize (the staging is one by design).

## Status

recorded, NOT fixed. The concurrency test is the record (it goes red on the 500s).

_STATUS: FIXED. `Plugins` takes an op lock; a concurrent install/remove is refused 409
conflict (`begin_install`/`begin_remove` try_lock). Real: concurrent-install test 6/6 (was 5x 500)._
