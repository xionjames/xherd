---
description: xherd — carry out the approved decisions on the pull request comments
---
# Apply the pull request decisions

Read the xherd rules in `{{dir}}/rules.md` first and follow them.

1. Read `.xherd/spec/pr-review.md`, the user's approved decisions, and
   `.xherd/spec/memory.md`. Do only what they say; the user may have edited a
   decision before approving.
2. For each comment decided `fix`, add a task to `.xherd/spec/tasks.md` with
   `source: pr`, the files it changes and a `verify` command when one fits,
   then run `xherd tasks sync`.
3. Implement those tasks one by one as `{{dir}}/next.md` describes, each with
   its own commit gate. If there are none, skip to step 4.
4. When they are all done, push the branch (`git push`): only your own
   branch, never a force-push. The pull request updates by itself.
5. Comments decided `reply` have their drafted answer in `pr-review.md`: say
   in your last message which ones the user should post, with the text. Do not
   post anything yourself.
6. Update `memory.md` and stop.
