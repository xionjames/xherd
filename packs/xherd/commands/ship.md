---
description: xherd — draft the pull request; once approved, push and open it as a draft
---
# Ship

Arguments: `{{args}}`

Read the xherd rules in `{{path:rules}}` first and follow them.

1. `xherd stage enter ship`
2. If `.xherd/spec/pr.md` exists, try `xherd gate open ship`.
   - Refused with `already_approved`: the user approved it. Go to step 7.
   - Accepted: the gate is open again for the current text. Stop and wait.
3. Otherwise draft it. xherd looks up the repository's pull-request
   templates when you enter this stage: read `.xherd/spec/pr-templates.json`
   (`{"templates": [{path, name, body}]}`; an empty list means the repository
   has none). If the file is not there yet, wait a moment and read it again;
   if it still is not, look in the usual places yourself
   (`.github/pull_request_template.md`, `.github/PULL_REQUEST_TEMPLATE/*.md`,
   `docs/`, the root, `.gitlab/merge_request_templates/*.md`). With several
   templates, pick the fitting one and name its `path` in the frontmatter.
4. Write `.xherd/spec/pr.md` in the pr format: the template's headings first,
   the essentials (summary, requirements covered, key decisions, testing,
   assumptions) placed in them. In a quick job (`Flow: xherd / quick` in
   `.xherd/instructions.md`) there is no `spec.md` or `tasks.md`: write the
   summary from the commits (`git log` since the base), `ticket.md` and
   `memory.md`, with "Not applicable" for the requirements. For a bug fix (`flow: bugfix` in `spec.md`),
   name the reproduction test and how it failed before the fix (from the
   evidence of `T01` in `tasks.md`). It becomes the pull request's body, so it
   holds only what belongs there; do not update the ticket yourself: xherd
   offers the user a summary to post once the PR is open.
5. Only when `.xherd/instructions.md` has no "Agent instructions" section (the
   job has no saved agent): xherd keeps what the user told you in
   `.xherd/spec/user-notes.md`. If it shows something the user had to repeat
   or correct that would help the next job of this kind (conventions, how to
   test, what to avoid), write those lines to `./.xherd/agent-draft.md`, nothing
   about this task, and offer them:
   `xherd agent propose --name shop-api --description "Feature work on the shop API" --instructions-file ./.xherd/agent-draft.md`.
   The user decides later whether to save an agent; skip this when there is
   nothing general to say.
6. `xherd gate open ship`, then stop and wait for the user.
7. Approved: check that the repository has a remote with
   `git remote get-url origin`. With none, do not push or open a pull
   request: skip to step 9 and tell the user the job page offers to add a
   remote (and open the pull request) or to finish locally. Otherwise push the
   branch (`git push -u origin HEAD`) and open a **draft**
   pull request with the host's CLI, using the title and the body of
   `pr.md` without its frontmatter (for GitHub:
   `gh pr create --draft --title "<title>" --body-file <file>`).
8. Report it: `xherd pr opened --url https://github.com/acme/app/pull/42`
   (with the real URL).
9. Update `memory.md` (write the pull request URL in its State section) and stop.
