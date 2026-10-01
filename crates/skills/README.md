# agent-hub-skills

**Not wired.** Skills are a top-level mechanism whose **content comes from a plugin**, delivered
the way extensions are (workspace/session layering), per the decided direction. An earlier version
was a hub-authored store with real `PUT`/`DELETE` mutators - the excluded model (TASK-048 C2).
Those mutators are stopped: every `/v1/skills` route answers `501 not_implemented` until the
plugin-source, layered model is real. A success endpoint over the wrong model is not honest
unavailability.
