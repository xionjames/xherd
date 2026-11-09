---
description: xherd — review the whole branch against the spec
---
# Review

Read the xherd rules in `{{dir}}/rules.md` first and follow them.

Review as if you had not written the code: judge the branch by the approved
documents and the diff, not by what you remember of the work. When you can
run a sub-agent with a fresh context, give it this review to do.

1. `xherd stage enter review`
2. Read `spec.md`, `design.md` (if any) and `tasks.md`, and the full branch
   diff against its base (the `Base` line in `.xherd/instructions.md`):
   `git diff <base>...HEAD`.
3. Check every requirement is met and tested, look for bugs, missing error
   handling, security problems, leftovers (debug output, TODOs), and run the
   repository's tests and linters.
4. Write each finding as `### F01: title` with `- severity:`, `- location:`
   and, once it has a task, `- fixed: T05` (the review format in the
   rules). Each finding that must be fixed becomes a new task in
   `tasks.md` with `source: review`; run `xherd tasks sync` and implement them one by
   one as `{{dir}}/next.md` describes (each with its own commit gate). When
   they are all done, come back to this review and run
   `xherd stage enter review` again: xherd counts each return as a review
   round, and holds the gate for the user after too many.
5. Write `.xherd/spec/review.md` in the review format, and update
   `memory.md`.
6. `xherd gate open review`
7. Stop and wait for the user.
