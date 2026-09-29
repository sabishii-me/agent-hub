# agent-hub-skills

The skills domain: the hub's side. A skill is a **directory**, not a record; its id is the
directory name, and the hub is a courier - it stores bytes, it never interprets them.

- `GET /v1/skills` - `{id, hasManifest, files}` per directory.
- `PUT/GET /v1/skills/{id}/files/{file...}` - write/read one file.
- `DELETE /v1/skills/{id}`.

Before a harness's process starts the hub **installs** the effective skills into
`<DATA_DIR>/agents/<harness>/skills` and hands that directory over as
`AGENT_HUB_INSTALLED_SKILLS_DIR`; the adapter points its harness at it with discovery off (pi/jouzu
`--no-skills --skill`, dsh `DSH_AGENTS_HOME`). Placement is the hub's; it is **not** an
authorization boundary - it keeps the user's own skill directories out of a managed session.

A path segment that is empty, `.`, `..`, contains a separator or `:` is an invalid input, so a
file can never escape the skills root.

## Tests

`cargo test -p agent-hub-skills` runs the HTTP store (write/list/read/delete), a traversal
refusal, and the contract error code for a missing skill.
