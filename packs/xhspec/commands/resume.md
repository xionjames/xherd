---
description: xhSpec — reload the state and continue where the work stopped
---
# Resume

Read the xhSpec rules in `{{dir}}/rules.md` first and follow them.

1. Read `.xherd/instructions.md` (when under xherd), `.xherd/HANDOFF.md` if it
   exists, `.xherd/spec/memory.md`, and the documents in `.xherd/spec/`.
2. Check `git status` and `git log` against `commits.md` and `tasks.md`.
3. Continue the current stage with its command (`{{prefix}}design`,
   `{{prefix}}tasks`, `{{prefix}}next`, `{{prefix}}review`, `{{prefix}}ship`):
   read that command's file in `{{dir}}` and follow it. If the stage waits at
   a gate that is not approved yet, declare it again and stop.
