---
description: xhSpec — break the approved spec and design into tasks
---
# Feature: tasks

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. `"$XHERD_BIN" stage enter tasks`
2. Re-read `.xherd/spec/spec.md` and, if it exists, `.xherd/spec/design.md`.
3. Write `.xherd/spec/tasks.md` in the tasks format: small tasks, each one
   commit, with `refs`, the exact `files` it may change, `depends`, and a
   `verify` command when one exists. A test that fails before the change
   comes first when that is practical.
4. Check that xherd can read it: `"$XHERD_BIN" tasks sync`. Fix the file
   until the call is accepted.
5. Update `.xherd/spec/memory.md`.
6. `"$XHERD_BIN" gate open tasks`
7. Stop and wait for the user.
