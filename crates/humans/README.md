# agent-hub-humans

The humans domain (`ARCHITECTURE` §3): approvals and questions - a harness asking a person to
decide. The hub holds the pending decisions as control state; the adapter initiates
(`approval_need` / a question) and the decision travels back as its reply.

- `GET /v1/sessions/{id}/approvals`, `POST .../approvals/{aid}` with a `decision`.
- `GET /v1/sessions/{id}/questions`, `POST .../questions/{qid}` with `answers`.

**Fail closed.** A decision must be one the harness **offered** (a `select`'s options); anything
else is `validation_failed`, never accepted. An unanswered approval is denied by its deadline - the
hub never allows by default. An approval is distinct from a question: an approval permits one
action; a question asks for data the harness reads back.

## Tests

`cargo test -p agent-hub-humans` decides an approval over HTTP, refuses an option that was not
offered, carries a question's answers back, and returns the contract code for an unknown approval.
