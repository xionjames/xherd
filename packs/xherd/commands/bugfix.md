---
description: xherd — start a bug fix: read the ticket, find the cause, write the analysis and its tasks
argument-hint: [<ticket key or URL>] <the bug>
---
# Bug fix: analysis

Arguments: `{{args}}`: the ticket when there is one (a key such as `PAY-412`
or `#12`, or an issue URL; `.xherd/instructions.md` says which), then a
description of the bug.

Read the xherd rules in `{{path:rules}}` first and follow them.

1. Mark the stage: `xherd stage enter analysis`.
2. If there is a ticket, read it with the tracker tools you have (MCP server
   or CLI). If there is none, or you have no tools for it, use the
   description given above. Write
   `.xherd/spec/ticket.md` (create the folder if needed).
3. Investigate: read the code, the logs and the tests around the bug, and run
   existing tests or commands to see it. **Change no file of the
   repository** in this step (not even a temporary test or debug line): only
   the documents in `.xherd/spec/` are written now. Find the root cause; when
   you cannot be sure, give your best hypothesis, how sure you are, and what
   would confirm it.
4. Write `.xherd/spec/spec.md` in the bug form of the spec format
   (`flow: bugfix`, `stage: analysis`): symptom, steps to reproduce, root
   cause, fix approach, the reproduction test (which file, what it asserts,
   the one command that runs it), the expected behaviour as numbered
   requirements, acceptance, out of scope, open questions, assumptions.
5. Write `.xherd/spec/tasks.md` in the bug shape of the tasks format:
   - `T01` adds the reproduction test only: `files:` lists the test file(s)
     and nothing else, and it has **no `verify:`** (the test fails until the
     fix, and xherd would refuse to mark the task done).
   - Then the fix task(s), depending on `T01`: `files:` lists the production
     files to change and **never the test file of `T01`**; `verify:` is the
     reproduction command, which must pass once fixed.
   Keep the list short: usually one fix task.
6. Check that xherd can read it: `xherd tasks sync`. Fix the file until the
   call is accepted.
7. Write `.xherd/spec/memory.md`, with the code map of what you explored
   (the files of the cause and of the test, how to build and run the tests).
8. Fresh-context check. Only when `.xherd/instructions.md` has a "Fresh-context
   check" section that lists `analysis`; otherwise skip this step. Run a
   sub-agent with a fresh context: give it `spec.md`, `tasks.md`, `ticket.md`
   and the files they name, never `memory.md` or this conversation, and ask it
   to restate in a short paragraph the bug and the fix, then list at most five
   points where the root cause is not backed by the code, the reproduction test
   would not catch the bug, or the tasks do not cover the fix; no style notes.
   It writes no file. Fix what you agree with in `spec.md` and `tasks.md`
   (then run `xherd tasks sync`); move what only the user can decide to
   `## Open questions` of `spec.md`; add a line to `## Assumptions` saying a
   fresh-context check ran and what it changed. When you have no sub-agent,
   write "Fresh-context check skipped: no sub-agent." there instead. Run the
   check once for each time you open the gate.
9. `xherd gate open analysis`: one gate over `spec.md` and `tasks.md`. Put
   every question for the user in the open questions section of `spec.md`,
   one list item each: that is where xherd reads them and where the user
   answers them.
10. Stop and wait for the user.
