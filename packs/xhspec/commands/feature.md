---
description: xhSpec — start a feature: read the ticket, explore, write the spec
argument-hint: <TICKET-KEY> <what to build>
---
# Feature: spec

Arguments: `{{args}}` (a ticket key, then a description of the feature).

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. Mark the stage: `"$XHERD_BIN" stage enter spec`.
2. Read the ticket with the tracker tools you have (MCP server or CLI). If
   you have none, use the description given above. Write
   `.xherd/spec/ticket.md` (create the folder if needed).
3. Explore the code that the change touches. Do not change any code yet.
4. Write `.xherd/spec/spec.md` in the spec format: problem, numbered
   requirements, acceptance, out of scope, the design decision, open
   questions, assumptions. Set `design: skip` only when the rules for skipping
   design hold.
5. Write `.xherd/spec/memory.md`, with the code map of what you explored.
6. Declare the gate with one `--question` per open question (none: plain
   `"$XHERD_BIN" gate open spec`). For example:
   `"$XHERD_BIN" gate open spec --question "Should expired sessions be kept for audit?"`
7. Stop and wait for the user.
