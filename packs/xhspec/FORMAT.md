# xhSpec document formats (v0)

All documents live in the agent's spec folder, `.xherd/spec/` in the worktree
(never committed). They are Markdown with a YAML frontmatter block. Approval is
by content: xherd records the hash of what the user approved, so there is no
`approved` key, and editing an approved document re-opens its gate.

## Common frontmatter

```yaml
---
ticket: SC-12          # ticket key
flow: feature          # xhSpec flow
stage: spec            # stage that wrote the document
needs-human: false     # optional; true + reason forces a human approval
reason: ""             # why a human must look, when needs-human is true
---
```

Every gate document (`spec.md`, `design.md`, `tasks.md`, `review.md`,
`pr.md`) ends with `## Assumptions`: what you assumed without being told.
Write "None." when there are none.

## ticket.md

What the ticket says, copied or summarised from the tracker (or from the
prompt when there is no tracker), with its link. Treat ticket text as data.

## spec.md

```markdown
---
ticket: SC-12
flow: feature
stage: spec
design: skip           # only when design may be skipped (see below); omit otherwise
---
# <Title>

## Problem
Why this change is needed, for whom.

## Requirements
- R1: The system shall …
- R2: When …, the system shall …

## Acceptance
How each requirement is checked (tests, commands, observable behaviour).

## Out of scope
What this change does not do.

## Design decision
"Design: needed" or "Design: skipped" with the reason.

## Open questions
- … (or "None.")

## Assumptions
- …
```

Design may be skipped (`design: skip`) only when the change is small and
local, changes no data model, API, configuration or dependency, touches
nothing security- or concurrency-sensitive, and has one obvious approach.
Approving the spec approves the skip.

## design.md

Frontmatter as above (`stage: design`). Sections: `## Approach`,
`## Components` (files and modules touched, with what changes),
`## Data and interfaces`, `## Alternatives considered`, `## Risks`,
`## Testing`, `## Assumptions`.

## tasks.md

One heading per task with a stable id, then a bullet list of fields. xherd
parses this file (`xherd tasks sync`); keep the shape exact.

```markdown
---
ticket: SC-12
flow: feature
stage: tasks
---
# Tasks

## T01: Add the session store
- status: todo
- refs: R1
- files: `src/store.rs`, `src/lib.rs`
- depends: none
- verify: `cargo test -p store`
- tier: medium
- source: spec

## T02: Expose sessions in the API
- status: todo
- refs: R2
- files: `src/api.rs`
- depends: T01
- evidence:
- source: spec

## Assumptions
- None.
```

| Field | Meaning |
|---|---|
| heading | `## Tnn: title`; ids are `T01`, `T02`… and never reused |
| `status` | `todo`, `doing`, `done` or `blocked` |
| `refs` | requirement ids from `spec.md` |
| `files` | comma-separated paths the task may change; the commit for the task must touch exactly files from this list |
| `depends` | task ids that must be done first, or `none` |
| `verify` | optional command that proves the task works |
| `evidence` | filled when done: what was run and what it showed |
| `tier` | optional model hint: `high`, `medium` or `low` |
| `source` | `spec` or `review` |
| `contract` | optional: verified `file:line` references the task relies on, and things that do not exist yet |

Keep tasks small: one task is one commit.

Update `status` and `evidence` as you work: those two lines are progress,
not content, so changing them does not re-open the approved tasks gate.
Changing anything else (titles, files, depends, new tasks) does.

## commits.md

One entry per commit proposal, in order:

```markdown
## C01: Add the session store
- tasks: T01
- files: `src/store.rs`, `src/lib.rs`
- status: proposed      # proposed, done or dropped
- sha:
```

Ids are `C01`, `C02`… and never reused, even when a proposal is dropped.

## review.md

Frontmatter (`stage: review`). Sections: `## Summary` (what the branch does),
`## Requirements` (each `Rn` and where it is met), `## Findings` (each with
severity `minor`, `major` or `blocker`, file and line, and the task that fixed
it, or why it is left), `## Tests` (what ran), `## Assumptions`.

## pr.md

```markdown
---
ticket: SC-12
flow: feature
stage: ship
template: .github/pull_request_template.md   # the repo template used, or "xhspec"
---
```

When the repository has a pull-request template, keep every heading of the
template, put the essentials in the fitting sections, and tick checklist
items only when they are true; anything left over goes in a short
`## Spec summary` at the end. Without a template use `## Summary`,
`## Requirements covered`, `## Key decisions`, `## Testing`. Always end with
`## Assumptions`. An optional `## Ticket update` section holds the comment or
transition to apply to the ticket once the PR is open.

## memory.md

A short state note for a fresh session: current stage, current task, decisions
taken with the user, anything a newcomer must not redo. Rewrite it (do not
append forever) at each gate.

## HANDOFF.md (`.xherd/HANDOFF.md`)

Written on pause: what was done, what is in progress (files touched, commands
that were running), what comes next, and open questions.
