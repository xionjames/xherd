# Configuration

xherd keeps its settings in `config.json` in the data root (`XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`; in the container, `/var/lib/xherd-daemon/data`). Secrets (git tokens, plugin secrets) are in `secrets.json`, mode `0600`, never in `config.json`.

Most settings are edited in the web UI (Settings) and saved to the file. **The running daemon keeps the configuration it started with**, so a change applies at the next daemon start; the UI says "restart needed" when saved and running differ. These apply at once: `welcomed`, `disabledPlugins`, the commit identity (`gitUserName`, `gitUserEmail`, `gitIdentities`) and git access (`gitHosts` and their tokens).

Files are written atomically. Keys this version does not know are kept when saving. A `config.json` newer than the running xherd is refused with a message to update.

## Reference

| Key | Default | Meaning |
|---|---|---|
| `port` | `4740` | HTTP port |
| `bind` | `127.0.0.1` | Bind address. In the container `XHERD_BIND` overrides it (`0.0.0.0`); the port is published on the host's loopback only |
| `workspace` | `$XHERD_WORKSPACE`, else `~/xherd` | Absolute path holding `repos/` and `worktrees/` |
| `tmuxSocket` | default server | tmux socket name (`tmux -L`) |
| `maxConcurrentAgents` | `4` | Live agents at once (1–64). The rest wait in *queued* |
| `pauseTimeoutSec` | `120` | How long a pause waits for the agent to stop (5–3600) |
| `eventRetentionDays` | `90` | Events older than this are pruned daily (≥ 1) |
| `idleAlertMinutes` | `15` | Notify when an agent has been idle this long; `0` = off |
| `gateParkMinutes` | `30` | A gate waiting this long parks the agent (frees its slot, stays in *Needs you*); `0` = never |
| `freshSessions` | `true` | Each approved gate starts a new runtime session; `memory.md` carries over |
| `ntfyServer` / `ntfyTopic` | unset | Also send every notification to this ntfy topic (both needed; messages carry agent names and reasons, in plain text). Applies at once; Settings → Notifications |
| `maxSessionMinutes` | `0` | End a session and pause the agent after this many minutes; `0` = off (no handoff is written first) |
| `prPollMinutes` | `5` | How often a watched pull request is polled; `0` = off |
| `maxReviewLoops` | `3` | Review → fix rounds before the review gate is held for a human; `0` = no limit |
| `autonomy` | `ask` | Default for new agents: `ask` or `important`. `never` is not available yet |
| `maxAutoApprovalsInRow` | `10` | After this many automatic approvals in a row the next gate waits for you; `0` = no limit |
| `agentTokenBudget` | `0` | Tokens one agent may use; `0` = off. 80 % notifies once, 100 % pauses the agent at its next stop |
| `dailyTokenBudget` | `0` | Same, over all agents per UTC day |
| `maxDiffLines` | `1000` | A commit proposal with more changed lines is held for a human (1–1,000,000) |
| `sensitivePaths` | `[]` | Globs: a commit proposal touching one is held. `*` stays within a folder, `**` crosses folders, a pattern without `/` matches a file name anywhere |
| `gitUserName`, `gitUserEmail` | none | Commit identity of agents (both needed) |
| `gitIdentities` | `[]` | Identities by remote: `{scope: "github.com/my-company", name, email}`, matched on the remote URL over HTTPS or SSH |
| `gitHosts` | `[]` | Git access per host: protocols, where the HTTPS token comes from (`gh`, `glab`, `xherd`, `none`), username, CA bundle, remote rewriting |
| `pluginSettings` | `{}` | Non-secret settings by plugin name (for example `github.gh`, the `gh` binary) |
| `disabledPlugins` | `[]` | Plugins switched off. At least one of each kind stays on |
| `updateCheck` | `true` | Check for new versions |
| `welcomed` | `false` | The first-run Welcome screen was completed or skipped |
| `userToken` | generated | Token of the local user (the CLI and local browsers). Do not share it |

The "held for a human" settings (`maxDiffLines`, `sensitivePaths`, the secret tripwire) set *needs human* on the gate: autonomy never approves it, and the UI shows why. They are global; per-repository profiles are not built yet.

## What the UI can change

Settings → General, Gates & autonomy, Notifications, Git access and Plugins cover: `maxConcurrentAgents`, `pauseTimeoutSec`, `workspace`, `eventRetentionDays`, `idleAlertMinutes`, `gateParkMinutes`, `freshSessions`, `autonomy`, `maxAutoApprovalsInRow`, `ntfyServer`, `ntfyTopic`, the commit identities, git hosts and plugin settings. The remaining keys (`maxSessionMinutes`, `prPollMinutes`, `maxReviewLoops`, the budgets, `maxDiffLines`, `sensitivePaths`) are accepted by `PUT /api/v1/config` with range checks but have no form yet; edit `config.json` and restart.

## Per-agent files

Every worktree has a `.xherd/` folder (excluded from git through the repository's `info/exclude`):

| Path | Contents |
|---|---|
| `agent.json` | The agent's identity and settings |
| `instructions.md` | What the agent is told about xherd and its autonomy level |
| `spec/` | The documents of the flow (`spec.md`, `design.md`, `tasks.md`, `commits.md`, `review.md`, `pr.md`, …) and `memory.md` |
| `ledger/approvals.jsonl` | Every approval with the content hash. The file wins over the database on conflict |
| `toolchain.json` | The tools the worktree's version files ask for |
| `hooks/` | The worktree's git hooks, including the pre-push guard |
| `commit-error.txt` | Written when git refuses an approved commit |
