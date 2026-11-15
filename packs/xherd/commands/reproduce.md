---
description: xherd — add the test that reproduces the bug, see it fail, and propose its commit
---
# Bug fix: reproduce

Read the xherd rules in `{{path:rules}}` first and follow them.

1. `xherd stage enter reproduce`
2. Read `.xherd/spec/memory.md`, `.xherd/spec/spec.md` (its reproduction
   test section) and `.xherd/spec/tasks.md`. The reproduction task is `T01`.
3. If `T01` is already `done` (the analysis was approved again, or the work
   resumed here), do not start it again and do not touch the test: when
   `commits.md` has no commit for `T01` yet and the test was not waived with
   the user, go to step 8; otherwise follow `{{path:next}}` and stop here.
4. When an automated test is not practical here (no test harness, it needs
   real hardware or a third-party service…), do not skip this step: before
   writing anything, ask
   `xherd ask "The bug needs a real printer to reproduce: may I continue without an automated test?"`
   and stop. With the user's go-ahead, close the task with their answer as
   evidence (`xherd task done T01 --evidence "No automated test: agreed with the user (no printer in CI)"`),
   propose no commit, and continue with `{{path:next}}`.
5. `xherd task start T01`, then write the test the analysis describes, in
   the files `T01` lists. Change **no production code**: the test must show
   the bug as it is today.
6. Run it with the reproduction command and read the output. It must fail
   **for the reason the analysis gives**: the assertion about the bug fails.
   A compile error, a missing fixture, a timeout or any other failure does
   not count: fix the test and run it again. If it passes, the analysis is
   wrong or the test misses the bug: fix the test; if you cannot make it
   show the bug, explain with
   `xherd ask "The test passes on the current code: the cause may be elsewhere. How should I proceed?"`
   and stop.
7. Mark it done with the failing run as evidence: the command, its exit
   status and the lines that show the failure, on one line:
   `xherd task done T01 --evidence "cargo test -p store expired_session -> exit 101: assertion failed: session.is_expired() (left: false, right: true)"`
8. Choose the next unused commit id (see `commits.md`, usually `C01`), add
   the entry (heading, `tasks`, `files`) to `.xherd/spec/commits.md`, and
   propose it with the test files only:
   `xherd commit propose C01 --subject "Add a failing test for expired sessions" --message "Add a failing test for expired sessions: a session past its expiry is still accepted (#12)" --files tests/session.rs --tasks T01`
   The message names the bug, in the repository's conventions. Do not commit
   yourself.
9. Rewrite `.xherd/spec/memory.md` (state, the reproduction command, code
   map for the fix), then stop and wait for the user.
