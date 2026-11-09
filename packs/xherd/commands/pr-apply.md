---
description: xherd — carry out the approved decisions on the pull request comments
---
# Apply the pull request decisions

Read the xherd rules in `{{dir}}/rules.md` first and follow them.

1. Read `.xherd/spec/pr-decisions.json` and `.xherd/spec/memory.md`. The file
   holds the final decision for every comment: the user's, or your proposal
   where they left it. Do only what it says. `note` is the user's or your
   reason; `posted: true` means xherd already posted that reply.
2. For each comment decided `fix`, add a task to `.xherd/spec/tasks.md` with
   `source: pr`, the files it changes and a `verify` command when one fits,
   then run `xherd tasks sync`.
3. Implement those tasks one by one as `{{dir}}/next.md` describes, each with
   its own commit gate. If there are none, skip to step 4.
4. When they are all done, push the branch (`git push`): only your own
   branch, never a force-push. The pull request updates by itself.
5. Replies decided `reply` were posted by xherd. A `fix` may carry a `reply`
   draft: xherd does not post it, so give it to the user in your last
   message, once the fix is pushed. Never post anything yourself.
6. Update `memory.md` and stop.
