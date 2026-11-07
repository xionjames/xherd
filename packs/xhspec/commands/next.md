---
description: xhSpec — implement the next ready task and propose its commit
---
# Implement the next task

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. `xherd stage enter implement`
2. Read `.xherd/spec/memory.md` and `.xherd/spec/tasks.md`. The next task is the first one with status
   `todo` whose `depends` are all `done`.
3. If no task is left to do (all `done`), follow the review command in
   `{{dir}}/review.md` instead and stop here.
4. If tasks remain but none is ready (blocked ones), say why with
   `xherd ask "T03 waits for T02, which is blocked: how should I proceed?"`
   and stop.
5. Start it: `xherd task start T01` (with the real id), and set its
   status to `doing` in `tasks.md`.
6. Implement it, changing only the files listed in its `files`. If it needs
   another file, add it to `files`, run `xherd tasks sync`, and say so
   in the commit proposal. If the task cannot be done, run
   `xherd task block T01 --reason "The API has no endpoint for this"`
   and stop.
7. Verify: run its `verify` command and the relevant tests. Fix until green.
8. Mark it done with what proved it:
   `xherd task done T01 --evidence "cargo test -p store: 12 passed"`,
   and set `status: done` and `evidence:` in `tasks.md`.
9. Choose the next unused commit id (`C01`, `C02`… see `commits.md`), add the
   entry to `.xherd/spec/commits.md` with `status: proposed`, and propose it:
   `xherd commit propose C01 --subject "Add the session store" --files src/store.rs,src/lib.rs --tasks T01`
   Do not commit yet.
10. Rewrite `.xherd/spec/memory.md` (state, decisions, code map for the next
    task), then stop and wait for the user.
