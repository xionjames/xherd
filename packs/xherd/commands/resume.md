---
description: xherd — reload the state and continue where the work stopped
---
# Resume

Read the xherd rules in `{{path:rules}}` first and follow them.

1. Read `.xherd/instructions.md` (when under xherd), `.xherd/HANDOFF.md` if it
   exists, `.xherd/spec/memory.md`, and the documents in `.xherd/spec/`.
2. Check `git status` and `git log` against `commits.md` and `tasks.md`.
3. Continue the current stage with its command: read its file and follow it
   (`{{prefix}}design`: `{{path:design}}`, `{{prefix}}tasks`: `{{path:tasks}}`,
   `{{prefix}}next`: `{{path:next}}`, `{{prefix}}review`: `{{path:review}}`,
   `{{prefix}}ship`: `{{path:ship}}`). If the stage waits at a gate that is not
   approved yet, declare it again and stop.
