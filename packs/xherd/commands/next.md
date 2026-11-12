---
description: xherd — implement the next ready task and propose its commit
---
# Implement the next task

Read the xherd rules in `{{path:rules}}` first and follow them.

1. `xherd stage enter implement`
2. Read `.xherd/spec/memory.md` and `.xherd/spec/tasks.md`. The next task is the first one with status
   `todo` whose `depends` are all `done`.
3. If no task is left to do (all `done`), follow the review command in
   `{{path:review}}` instead and stop here.
4. If tasks remain but none is ready (blocked ones), say why with
   `xherd ask "T03 waits for T02, which is blocked: how should I proceed?"`
   and stop.
5. Start it: `xherd task start T01` (with the real id). xherd writes the
   `status:` and `evidence:` lines of `tasks.md` itself; do not edit them.
6. Implement it, changing only the files listed in its `files`. If it needs
   another file, add it to `files`, run `xherd tasks sync`, and say so
   in the commit proposal. If the task cannot be done, run
   `xherd task block T01 --reason "The API has no endpoint for this"`
   and stop.
7. Verify: run its `verify` command and the relevant tests. Fix until green.
8. Mark it done with what proved it:
   `xherd task done T01 --evidence "cargo test -p store: 12 passed"`.
   If the task has a `verify`, xherd runs it too and refuses the call while
   it fails: fix and call again.
9. Choose the next unused commit id (`C01`, `C02`… see `commits.md`), add the
   entry (heading, `tasks`, `files`) to `.xherd/spec/commits.md`, and propose
   it (xherd writes its `status:` and `sha:` lines):
   `xherd commit propose C01 --subject "Add the session store" --message "Add the session store: sessions live in SQLite, one row per login (T01)" --files src/store.rs,src/lib.rs --tasks T01`
   The message is the full commit message, in the repository's conventions;
   the user can edit it, and xherd commits with it once approved. Do not
   commit yourself.
10. Rewrite `.xherd/spec/memory.md` (state, decisions, code map for the next
    task), then stop and wait for the user.
