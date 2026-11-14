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
   design hold.
5. Write `.xherd/spec/memory.md`, with the code map of what you explored.
6. `xherd gate open spec`. Put every question for the user in the open
   questions section, one list item each: that is where xherd reads them and
   where the user answers them.
7. Stop and wait for the user.
