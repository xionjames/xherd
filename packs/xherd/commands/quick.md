---
description: xherd — a small change: make it, test it and propose one commit
argument-hint: [<ticket key or URL>] <what to change>
---
# Quick change

Arguments: `{{args}}`: the ticket when there is one (a key such as `PAY-412`
or `#12`, or an issue URL; `.xherd/instructions.md` says which), then a
description of the change.

Read the xherd rules in `{{path:rules}}` first and follow them. This flow has
no `spec.md`, `design.md`, `tasks.md` or `review.md`: write none of them.

1. Mark the stage: `xherd stage enter implement`.
2. If there is a ticket, read it with the tracker tools you have (MCP server
   or CLI). If there is none, or you have no tools for it, use the
   description given above. Write `.xherd/spec/ticket.md` (create the folder
   if needed): what was asked, with its link. Promoting this job to the
   feature flow later starts from this file.
3. Look at the code the change touches (no more than that) and make the
   change. Keep it small: when it turns out to need a design, a data model
   or API change, or many files, do not push on. Ask
   `xherd ask "This is bigger than a quick change: it needs a new table. Should the job be promoted to the feature flow?"`
   and stop; the user promotes it from the job page.
4. Run the repository's relevant tests, linter and formatter, and fix until
   they pass.
5. Write `.xherd/spec/memory.md` (state, what was done, code map, how to
   build and test), short.
6. Choose the next unused commit id (`C01` for the first), add the entry
   (heading, `files`; no `tasks`) to `.xherd/spec/commits.md`, and propose
   it with exactly the files you changed:
   `xherd commit propose C01 --subject "Fix the typo in the README" --message "Fix the typo in the README: Instalation becomes Installation (#12)" --files README.md`
   The message is the full commit message, in the repository's conventions.
   Do not commit yourself.
7. Stop and wait for the user. When they ask for changes at this gate, revise
   the change and propose the commit again. When, at the ship gate, they ask
   for more code, make it, run the tests, and propose it as the next commit
   (`C02`…); after it is recorded, return to `{{path:ship}}`.
