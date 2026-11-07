---
description: xhSpec — commit an approved proposal, then continue
argument-hint: <commit id, e.g. C01>
---
# Commit an approved proposal

Arguments: `{{args}}` (the commit id).

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. Find the proposal in `.xherd/spec/commits.md`. The user approved exactly
   its files as they are now; do not change them.
2. Stage exactly those files (`git add -- <files>`; use `git rm` for deleted
   ones) and nothing else, then `git commit -m "<subject>"`. Use the
   repository's commit message conventions if it has any.
3. Get the sha with `git rev-parse HEAD`, then report it:
   `"$XHERD_BIN" commit done C01 --sha 3f2a9c1`
4. Record the sha and `status: done` in `commits.md`.
5. Continue: follow the command in `{{dir}}/next.md` (next task, or the review
   when every task is done).
