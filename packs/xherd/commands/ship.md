---
description: xherd — draft the pull request; once approved, push and open it as a draft
---
# Ship

Arguments: `{{args}}`

Read the xherd rules in `{{path:rules}}` first and follow them.

1. `xherd stage enter ship`
2. If `.xherd/spec/pr.md` exists, try `xherd gate open ship`.
   - Refused with `already_approved`: the user approved it. Go to step 6.
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
   assumptions) placed in them. It becomes the pull request's body, so it
   holds only what belongs there; do not update the ticket yourself: xherd
   offers the user a summary to post once the PR is open.
5. `xherd gate open ship`, then stop and wait for the user.
6. Approved: push the branch (`git push -u origin HEAD`) and open a **draft**
   pull request with the host's CLI, using the title and the body of
   `pr.md` without its frontmatter (for GitHub:
   `gh pr create --draft --title "<title>" --body-file <file>`).
7. Report it: `xherd pr opened --url https://github.com/acme/app/pull/42`
   (with the real URL).
8. Update `memory.md` and stop.
