---
description: xherd — answer the user's question, or turn the change they ask for into tasks and commit proposals
argument-hint: <the user's message>
---
# Ask

Arguments: `{{args}}`: a message from the user, typed in xherd's Ask box.

Read the xherd rules in `{{path:rules}}` first and follow them.

1. Read `.xherd/spec/memory.md` and the documents in `.xherd/spec/`, then the
   files the message is about.
2. If the message is a question, or asks for nothing in the repository to
   change, answer it in your reply and stop. Change no file.
3. If it asks for a change to the repository, never make it with `git commit`
   yourself: every change goes through a task and a commit proposal, so the
   user approves it and xherd lists it.
   - Add one task per change to `.xherd/spec/tasks.md`, after the last one
     (the next unused id), with the files it changes, `depends: none` and a
     `verify` command when one fits. Use `source: pr` when the branch already
     has a pull request (`memory.md` says so after the ship step), and
     `source: review` before it.
   - Run `xherd tasks sync`.
   - Implement them one by one as `{{path:next}}` describes, each with its own
     commit proposal (`xherd commit propose C07 --subject "Rename the retry helper" --message "Rename the retry helper to write_state (T12)" --files src/retry.rs --tasks T12`),
     then stop at its gate.
4. If the change is unclear, ask before writing a task:
   `xherd ask "Should the rename also cover the tests?"` and stop.
