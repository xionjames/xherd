---
description: xherd — write the design for the approved spec
---
# Feature: design

Read the xherd rules in `{{path:rules}}` first and follow them.

1. `xherd stage enter design`
2. Read `.xherd/spec/memory.md` and `.xherd/spec/spec.md` (approved). Open the
   files in the code map and those the spec names; look elsewhere only when
   they fall short, and add what you find to the code map.
3. Write `.xherd/spec/design.md` in the design format. Name real files and
   functions; verify each one exists before you cite it. Put every question
   for the user in its open questions section, one list item each.
4. Rewrite `.xherd/spec/memory.md`, with the code map for the tasks step.
5. Fresh-context check. Only when `.xherd/instructions.md` has a "Fresh-context
   check" section that lists `design`; otherwise skip this step. Run a
   sub-agent with a fresh context: give it `spec.md`, `design.md` and the files
   `design.md` names, never `memory.md` or this conversation, and ask it to
   restate in a short paragraph what the design does, then list at most five
   points where it is unclear, contradicts the spec or misses a requirement,
   and check that each file and function it cites exists; no style notes. It
   writes no file. Fix what you agree with in `design.md`; move what only the
   user can decide to `## Open questions`; add a line to `## Assumptions`
   saying a fresh-context check ran and what it changed. When you have no
   sub-agent, write "Fresh-context check skipped: no sub-agent." there instead.
   Run the check once for each time you open the gate.
6. `xherd gate open design`
7. Stop and wait for the user.
