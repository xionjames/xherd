---
description: xherd — merge the base branch into the pull request's branch and resolve the conflicts
argument-hint: <base, e.g. origin/main>
---
# Resolve conflicts with the base branch

Arguments: `{{args}}` (the base to merge, for example `origin/main`).

Read the xherd rules in `{{path:rules}}` first and follow them.

The pull request of this branch no longer merges cleanly into its base. You
merge the base into the branch, resolve the conflicts, and propose the merge
commit. The branch is already pushed: **merge, never rebase, reset or
force-push**, and never rewrite a commit that was pushed.

1. Read `.xherd/spec/memory.md` and the approved documents.
2. Run `git fetch origin`.
3. If `git merge-base --is-ancestor origin/main HEAD` succeeds (with the real
   base), the base is already in the branch: say so, update `memory.md` and
   stop. Push nothing.
4. Run `git merge --no-ff --no-commit origin/main` (with the real base).
5. Resolve each conflicted file (`git diff --name-only --diff-filter=U`),
   keeping the intent of both sides: the base's changes and this branch's
   work as the approved documents describe it. If git reported no conflict
   (already resolved on the host, or the status was stale), say so and go on.
6. Run the repository's build and tests (the code map in `memory.md` says
   how). Fix only what the merge broke, inside the merge.
7. `git add` the resolved files. Do not run `git commit`: xherd commits the
   merge when the user approves it.
8. Propose the merge with every staged path (`git diff --cached --name-only HEAD`)
   and no `--tasks`, using the next unused commit id from `commits.md` (add
   its entry there first, heading and `files`):
   `xherd commit propose C07 --subject "Merge origin/main into my-branch" --message "Merge origin/main into my-branch: keep both retry limits in src/client.rs" --files src/client.rs,src/lib.rs`
   The message says which conflicts there were and how you resolved them.
   Update `memory.md` and stop.
9. After the user approved it, `{{path:commit}}` brings you back here: push
   your own branch (`git push`, never a force-push), update `memory.md` and
   stop. xherd moves the job back to watching the pull request.
