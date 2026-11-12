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
5. `xherd gate open design`
6. Stop and wait for the user.
