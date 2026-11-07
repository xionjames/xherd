---
description: xhSpec — record the approved commit (or fix it), then continue
argument-hint: <commit id, e.g. C01>
---
# Record an approved commit

Arguments: `{{args}}` (the commit id).

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

xherd makes the commit itself when the user approves a proposal: it stages
exactly the proposed files and commits with the proposal's message (as the
user left it). You record it, or fix it when git refused.

1. Find the proposal in `.xherd/spec/commits.md`.
2. If `.xherd/commit-error.txt` exists, git refused the commit (a hook, a
   conflict…). Read it and fix what it says, changing only the proposed files,
   then commit them yourself: `git add -- <files>` (`git rm` for deleted
   ones) and `git commit -m "<message>"`, with no `--no-verify`. If the fix
   needs other files or a different change, say why with
   `xherd ask "The formatter wants to change src/db.rs too: may I include it?"`
   and stop. Delete the error file once the commit exists.
3. Get the sha with `git rev-parse HEAD` and check that
   `git show --name-only --format= HEAD` lists the proposal's files.
4. Report it: `xherd commit done C01 --sha 3f2a9c1` (harmless when xherd
   already recorded it).
5. Record the sha and `status: done` in `commits.md`.
6. Continue: follow the command in `{{dir}}/next.md` (next task, or the review
   when every task is done).
