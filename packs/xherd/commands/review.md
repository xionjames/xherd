---
description: xherd — review the whole branch against the spec
---
# Review

Read the xherd rules in `{{path:rules}}` first and follow them.

Review as if you had not written the code: judge the branch by the approved
documents and the diff, not by what you remember of the work. When you can
run a sub-agent with a fresh context, give it this review to do.

1. `xherd stage enter review`
2. Read `spec.md`, `design.md` (if any) and `tasks.md`, and the full branch
   diff against its base (the `Base` line in `.xherd/instructions.md`):
   `git diff <base>...HEAD`.
3. Check every requirement is met and tested, look for bugs, missing error
   handling, security problems, leftovers (debug output, TODOs), and run the
   repository's tests and linters. For a bug fix (`flow: bugfix` in
   `spec.md`), check that the reproduction test it names is on the branch and
   passes, and say so under `## Tests`.
4. Write each finding as `### F01: title` with `- severity:`, `- location:`
   and, once it has a task, `- fixed: T05` (the review format in the
   rules). Each finding that must be fixed becomes a new task in
   `tasks.md` with `source: review`; run `xherd tasks sync` and implement them one by
   one as `{{path:next}}` describes (each with its own commit gate). When
   they are all done, come back to this review and run
   `xherd stage enter review` again: xherd counts each return as a review
   round, and holds the gate for the user after too many.
5. Write `.xherd/spec/review.md` in the review format, and update
   `memory.md`.
6. Shared memory: `memory.md` dies with this job, the repository's
   `AGENTS.md` stays for every future job. If this work taught something a
   future job here would need (a build or test command, a convention, a
   trap that cost time) that `AGENTS.md` does not say yet, write the **whole**
   proposed file (today's `AGENTS.md`, or a new one, with your additions) to
   `.xherd/AGENTS.proposed.md`. Keep it small: only lasting facts a later job
   needs (commands, conventions, traps), each one short; no task history,
   status or changelog, no secrets; remove what this change made stale or
   wrong, and anything you noted in `memory.md` for it. Then run
   `xherd memory propose --reason "Adds the integration test command and the fixture rule"`.
   Nothing worth sharing: skip this step. Never commit `AGENTS.md` or
   `CLAUDE.md` yourself: the user accepts or dismisses the proposal and xherd
   commits it. Refused with `memory_declined`: skip this step.
   Agent memory: when `.xherd/instructions.md` has an "Agent memory" section
   (the job came from a saved agent) and the review found something lasting
   about how this agent should work (a habit, a trap, a preference the user
   showed), write the **whole** new memory (the section's text with your
   changes, at most 4 000 characters) to `.xherd/agent-memory.proposed.md` and
   run `xherd agent memory propose --reason "Always run the linter before proposing"`.
   Refused with `no_agent`, or nothing worth keeping: skip it. The user accepts
   or dismisses it; only jobs started later see the new memory.
7. `xherd gate open review`
8. Stop and wait for the user.
