# Add xherd support to your SDD

> Draft · protocol version 1 · See [`spec.md`](./spec.md) §8.6 and [`xap.md`](./xap.md).

xherd runs coding agents in parallel and stops them at approval gates. If you maintain a spec-driven development (SDD) method (slash commands, skills or scripts), a few calls let it run under xherd with gates, approvals, autonomy and per-step models. You don't need to write an xherd pack. Outside xherd the calls are skipped, so your SDD keeps working as before.

## Levels

| Level | You do | Users get |
|---|---|---|
| 0 | Nothing | xherd runs your commands as plain prompts: status, terminal, PR follow-up |
| **1** | Add `xherd` calls at the end of each step (this guide) | Gates, approvals, autonomy, a model/runtime per step; stages appear as your commands declare them |
| 2 | Also ship `xherd-flow.yaml` | Stage list up front, document validation, task panels, custom views |
| 3 | Let xherd install your prompts | `xherd install` sets your SDD up in every runtime |

## The rules

1. **Only under xherd.** Call xherd only when `XHERD_ENV` is `1`. Use `"$XHERD_BIN"` (an absolute path) to run it.
2. **Declare, then stop.** When a step's document is ready for a human, call `gate open` and stop. xherd shows it, and after approval it sends the next command.
3. **Say what comes next.** Pass `--next "<your next command>"`. It must start with one of the prefixes the user registered for your SDD (e.g. `/sdd-`).
4. **Say what kind of step it is.** Pass `--role` with one of `explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`. Users configure models, runtimes and autonomy per role, so this is what makes their settings apply to your SDD.
5. **Handle refusals.** Exit code 2 means xherd refused the call, and stderr says why in one line (e.g. a missing frontmatter field). Fix it and call again. Exit code 3 means the daemon is unreachable: stop and wait.
6. **Never approve.** Agents cannot approve; only the user (or xherd's autonomy rules) can.

## Snippets

### Prompt-based SDD (slash commands, skills)

Add a section at the end of each command file:

```markdown
## When running under xherd
If the environment variable `XHERD_ENV` is `1`:
- When you start this step, run `"$XHERD_BIN" stage enter <step-id> --role <role>`.
- When the document is ready for review, run
  `"$XHERD_BIN" gate open <step-id> --role <role> --doc <path> --next "<next command>"`,
  then stop and wait for approval.
- If the command exits with code 2, fix what it says and run it again. If it exits with 3, stop.
```

Optional additions:

```markdown
- Before working on a task: `"$XHERD_BIN" task start <ID>`; after verifying it: `"$XHERD_BIN" task done <ID> --evidence "<what you ran>"`.
- Before committing: `"$XHERD_BIN" commit propose <ID> --subject "<subject>" --files <a,b>` and stop; after committing: `"$XHERD_BIN" commit done <ID> --sha <sha>`.
- After opening the pull request: `"$XHERD_BIN" pr opened --url <url>`.
- Out-of-scope problems you notice: `"$XHERD_BIN" issue add --title "<title>" --severity minor|major|blocker --about <path>`.
```

### Script-based SDD (shell)

```bash
xherd() { [ "${XHERD_ENV:-}" = 1 ] && "$XHERD_BIN" "$@"; }

xherd stage enter sdd-spec --role spec
# … write the spec …
xherd gate open sdd-spec --role spec --doc "sdd/specs/$slug.spec.md" --next "/sdd-task sdd/specs/$slug.spec.md"
```

### Script-based SDD (Python)

```python
import os, subprocess

def xherd(*args: str) -> int:
    if os.environ.get("XHERD_ENV") != "1":
        return 0
    return subprocess.run([os.environ["XHERD_BIN"], *args]).returncode
```

## Example: one full step

```
/sdd-spec "Add login with passkeys"
  → writes sdd/specs/login-passkeys.spec.md
  → xherd gate open sdd-spec --role spec --doc sdd/specs/login-passkeys.spec.md --next "/sdd-task sdd/specs/login-passkeys.spec.md"
  → stops
User approves in xherd (or autonomy allows it)
  → xherd sends "/sdd-task sdd/specs/login-passkeys.spec.md" (in a fresh session if configured)
```

## Registering your SDD in xherd

Users add a dynamic flow in Settings → Flows with:

- **Name**, e.g. `my-sdd`.
- **Start command**, e.g. `/sdd-spec {prompt}` (`{key}` and `{prompt}` are filled in).
- **Allowed prefixes**, e.g. `/sdd-`.
- Whether your SDD needs its files **committed** (if it keeps state in the repo).

## Checking your integration

```
xherd sdd check ./path/to/your-sdd     # which level you reach, missing roles or --next
xherd sdd try ./path/to/your-sdd       # runs a fake agent through your calls
```

Tag your repository with the GitHub topic `xherd-sdd` so it appears in xherd's catalogue.
