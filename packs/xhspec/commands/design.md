---
description: xhSpec — write the design for the approved spec
---
# Feature: design

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. `"$XHERD_BIN" stage enter design`
2. Re-read `.xherd/spec/spec.md` (approved) and the code it touches.
3. Write `.xherd/spec/design.md` in the design format. Name real files and
   functions; verify each one exists before you cite it.
4. Update `.xherd/spec/memory.md`.
5. `"$XHERD_BIN" gate open design`
6. Stop and wait for the user.
