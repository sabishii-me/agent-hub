# 20261005-170000 — I queried /users and called an Organization an account

Status: RESOLVED (self-inflicted, no product impact).

## What happened
While designing the plugin contribution flow I wrote "`sabishii-me` is a GitHub ACCOUNT (not an
org)". I had queried `https://api.github.com/users/sabishii-me/repos`, which answers for both users
and orgs, so it did not distinguish them and I did not check `type`.

## The fact
`https://api.github.com/users/sabishii-me` -> `"type": "Organization"`.
`https://api.github.com/orgs/sabishii-me` -> 200, 24 public repos. `sabishii-me` IS an Organization.

## Why it matters (beyond the wording)
The design leans on org features for enforcement (a `registry-review` team, org rulesets, CODEOWNERS,
an org-required workflow). Those are available exactly because it is an org, so the design is
SIMPLER, not blocked. The wrong classification was my error.

## Root cause / rule
I used one endpoint and did not read the field that decides the question (`type`). Same pattern as
earlier misjudgements: read the field that answers the actual question, and confirm with a second
endpoint (`/orgs/<name>`).

## Fix
docs/tasks/registry-contribution-flow-design.md corrected: ORGANIZATION; org teams / rulesets /
required-workflow are all available. No code or contract impact.
