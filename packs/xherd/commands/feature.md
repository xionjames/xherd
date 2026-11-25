---
description: xherd — start a feature: read the ticket, explore, write the spec
argument-hint: [<ticket key or URL>] <what to build>
---
# Feature: spec

Arguments: `{{args}}`: the ticket when there is one (a key such as `PAY-412`
or `#12`, or an issue URL; `.xherd/instructions.md` says which), then a
description of the feature.

Read the xherd rules in `{{path:rules}}` first and follow them.

1. Mark the stage: `xherd stage enter spec`.
2. If there is a ticket, read it with the tracker tools you have (MCP server
   or CLI). If there is none, or you have no tools for it, use the
   description given above. Write
   `.xherd/spec/ticket.md` (create the folder if needed).
3. Explore the code that the change touches. Do not change any code yet.
4. Write `.xherd/spec/spec.md` in the spec format: problem, numbered
   requirements, acceptance, out of scope, the design decision, open
   questions, assumptions. Set `design: skip` only when the rules for skipping
   design hold. Size it to the change: a few lines per section for a small
   change, one requirement per behaviour, no restating the ticket or the
   code, "None." for a section with nothing to say. Keep every heading
   (xherd reads `## Open questions` and the `design:` key). When the change is
   small enough that the quick flow would have fitted, say so in
   `## Assumptions`; do not switch flows yourself.
5. Write `.xherd/spec/memory.md`, with the code map of what you explored.
6. Fresh-context check. Only when `.xherd/instructions.md` has a "Fresh-context
   check" section that lists `spec`; otherwise skip this step. Run a sub-agent
   with a fresh context: give it `spec.md`, `ticket.md` and the files `spec.md`
   names, never `memory.md` or this conversation, and ask it to restate in a
   short paragraph what the spec asks for, then list at most five points where
   it is unclear, contradictory or missing something; no style notes. It writes
   no file. Fix what you agree with in `spec.md`; move what only the user can
   decide to `## Open questions`; add a line to `## Assumptions` saying a
   fresh-context check ran and what it changed. When you have no sub-agent,
   write "Fresh-context check skipped: no sub-agent." there instead. Run the
   check once for each time you open the gate.
7. `xherd gate open spec`. Put every question for the user in the open
   questions section, one list item each: that is where xherd reads them and
   where the user answers them.
8. Stop and wait for the user.
