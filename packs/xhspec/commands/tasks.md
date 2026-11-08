---
description: xhSpec — break the approved spec and design into tasks
---
# Feature: tasks

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. `xherd stage enter tasks`
2. Read `.xherd/spec/memory.md`, `.xherd/spec/spec.md` and, if it exists,
   `.xherd/spec/design.md`. Trust the code map; open files only to size a task.
3. Write `.xherd/spec/tasks.md` in the tasks format: small tasks, each one
   commit, with `refs`, the exact `files` it may change, `depends`, and a
   `verify` command when one exists (one runnable shell command such as
   `cargo test -p store`, never a sentence: xherd runs it as written). A test that fails before the change
   comes first when that is practical.
4. Check that xherd can read it: `xherd tasks sync`. Fix the file
   until the call is accepted.
5. Rewrite `.xherd/spec/memory.md`, with the code map for the build steps.
6. `xherd gate open tasks`
7. Stop and wait for the user.
