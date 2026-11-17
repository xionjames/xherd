# Configuration

xherd keeps its settings in `config.json` in the data root (`XHERD_HOME`, else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd`; in the container, `/var/lib/xherd-daemon/data`). Secrets (git tokens, plugin secrets) are in `secrets.json`, mode `0600`, never in `config.json`.

Most settings are edited in the web UI (Settings) and saved to the file. A change saved through the UI or `PUT /api/v1/config` **applies at once** for every setting except `port`, `bind`, `maxConcurrentAgents`, `workspace`, `tmuxSocket` and `pluginSettings`, which are read at daemon start (the UI says "restart needed" when those differ). Live: the timeouts and limits (`idleAlertMinutes`, `gateParkMinutes`, `pauseTimeoutSec`, `freshSessions`, `maxSessionMinutes`, `prPollMinutes`, `maxReviewLoops`, `maxAutoApprovalsInRow`, the budgets, `maxDiffLines`, `sensitivePaths`), `autonomy`, `permissionMode`, `defaultRuntime`, `runtimeModels`, `openIn`, `toolchainDefaults`, `eventRetentionDays`, `ntfyServer`/`ntfyTopic`, `mutedNotifications`, `desktopNotifications`, `suggestAgents`, `welcomed`, `disabledPlugins` (the host, trust writer and toolchain plugins skip a disabled plugin), the commit identity and git access. A hand edit of `config.json` still needs a restart.

Files are written atomically. Keys this version does not know are kept when saving. A `config.json` newer than the running xherd is refused with a message to update.

## Reference

| Key | Default | Meaning |
|---|---|---|
| `port` | `4740` | HTTP port |
| `bind` | `127.0.0.1` | Bind address. In the container `XHERD_BIND` overrides it (`0.0.0.0`); the port is published on the host's loopback only |
| `workspace` | `$XHERD_WORKSPACE`, else `~/xherd` | Absolute path holding `repos/` and `worktrees/` |
| `tmuxSocket` | default server | tmux socket name (`tmux -L`) |
| `maxConcurrentAgents` | `4` | Live jobs at once (1–64). The rest wait in *queued* |
| `pauseTimeoutSec` | `120` | How long a pause waits for the job's handoff note (5–3600); then it pauses without one |
| `eventRetentionDays` | `90` | Events older than this are pruned daily (≥ 1) |
| `idleAlertMinutes` | `15` | Notify when a job has been idle this long; `0` = off |
| `gateParkMinutes` | `30` | A gate waiting this long parks the job (frees its slot, stays in *Needs you*); `0` = never |
| `freshSessions` | `true` | Each approved gate starts a new runtime session; `memory.md` carries over |
| `mutedNotifications` | `[]` | Notification kinds (`gate`, `blocked`, `idle`, `delivery`, `error`, `recovery`, `budget`, `limit`, `pr`, `pr-comments`, `todo`, `suggestion`) that are not sent anywhere. Applies at once |
| `desktopNotifications` | `false` | The daemon also shows system notifications (`osascript` on macOS, `notify-send` on Linux); not in the Docker image. Applies at once |
| `suggestAgents` | `true` | Offer to save a job's setup as an agent (a band on the job page and one `suggestion` notification) when a job with no agent is done or marked done, or when you start the same setup a third time (same repository, runtime, model, workflow, autonomy and tool permissions within 30 days). Never after a cancel or a failure. Applies at once; Settings → Gates & autonomy |
| `ntfyServer` / `ntfyTopic` | unset | Also send every notification to this ntfy topic (both needed; messages carry job names and reasons, in plain text). Applies at once; Settings → Notifications |
| `maxSessionMinutes` | `0` | End a session and pause the job after this many minutes; `0` = off (after the coding agent writes a handoff note, like Pause) |
| `prPollMinutes` | `5` | How often a watched pull request is polled; `0` = off |
| `maxReviewLoops` | `3` | Review → fix rounds before the review gate is held for a human; `0` = no limit |
| `autonomy` | `ask` | Default for new jobs: `ask` or `important`. `never` is not available yet |
| `openIn` | `{}` | Settings → Open in: `hidden` (entry ids left out of the menu: `vscode`, `cursor`, `zed`, `devcontainer`, `terminal`, `link:<id>`), `terminal` (the app Open in → Terminal starts: `terminal`, `iterm`, `ghostty`, `konsole`, `gnome-terminal`, `kitty`, `alacritty`, `wezterm`, `xfce4-terminal`, `foot`, `xterm`, `xdg-terminal-exec`, `x-terminal-emulator`; unset, or one not installed or not for that computer's OS = its default; Settings lists the ones the terminal handler found), `links` (your own links, `[{"id", "label", "url"}]`, shown as `link:<id>`; `{dir}` and `{job}` in the URL are replaced percent-encoded; for local apps only: the URL must start with a scheme such as `idea://`, and `http`, `https`, `ws`, `wss`, `ftp`, `javascript`, `data`, `vbscript`, `blob`, `file`, `about` and `xherd-term` are refused) and `droppedCommands` (labels of the `commands` of older versions, removed when the file was upgraded; Settings names them until you dismiss the notice, and it can only shrink) |
| `permissionMode` | `edits` | Tool permissions of new jobs: `ask` (asks before every tool use that needs permission), `edits` (accepts file edits), `auto` (the runtime's own checks decide), `all` (asks nothing; only accepted in the container) |
| `defaultRuntime` | `claude-code` | Runtime plugin of new jobs (`claude-code`, `opencode`, `codex`); also runs TODO condition checks. Must be a runtime plugin that is switched on |
| `runtimeModels` | `{}` | Runtime plugin → model of new jobs, e.g. `{"claude-code": "sonnet", "opencode": "opencode/big-pickle"}`; edited as *Default model* in each runtime's options (Settings → Plugins). No entry = the runtime's own default. Model names are letters, digits and `. _ : / @ - [ ]`; the runtime must take a model (`modelArgs` in its manifest) |
| `maxAutoApprovalsInRow` | `10` | After this many automatic approvals in a row the next gate waits for you; `0` = no limit |
| `agentTokenBudget` | `0` | Tokens one job may use; `0` = off. 80 % notifies once, 100 % pauses the job at its next stop |
| `dailyTokenBudget` | `0` | Same, over all jobs per UTC day |
| `maxDiffLines` | `1000` | A commit proposal with more changed lines is held for a human (1–1,000,000) |
| `sensitivePaths` | `[]` | Globs: a commit proposal touching one is held. `*` stays within a folder, `**` crosses folders, a pattern without `/` matches a file name anywhere |
| `gitUserName`, `gitUserEmail` | none | Commit identity of jobs (both needed) |
| `gitIdentities` | `[]` | Identities by remote: `{scope: "github.com/my-company", name, email}`, matched on the remote URL over HTTPS or SSH |
| `gitHosts` | `[]` | Git access per host (Settings → Git access): `host`, `protocols`, `provider` (a host plugin: `github`, `gitlab`, `bitbucket` or an external one; unset = the first whose host patterns claim the name), `credential` (where git gets the HTTPS token: `provider` = the provider's CLI helper such as `gh auth git-credential`, `xherd` = the token kept in `secrets.json`, `none` = git's own configuration), `username`, `caFile`, `preferHttps` / `preferSsh` (rewrite remotes) |
| `toolchainDefaults` | `{}` | Tool → version used when a worktree has no version file for it (`{"java": "21.0.5-tem"}`); set from Settings → Plugins → a toolchain. Version files always win |
| `pluginSettings` | `{}` | Non-secret settings by plugin name (for example `github.gh`, the `gh` binary). Removing a plugin with *forget* drops its entry |
| `disabledPlugins` | `[]` | Plugins switched off. At least one of each kind stays on |
| `updateCheck` | `true` | Check for new versions |
| `welcomed` | `false` | The first-run Welcome screen was completed or skipped |
| `userToken` | generated | Token of the local user (the CLI and local browsers). Do not share it |

The "held for a human" settings (`maxDiffLines`, `sensitivePaths`, the secret tripwire) set *needs human* on the gate: autonomy never approves it, and the UI shows why. They are global defaults; a repo profile (below) replaces `maxDiffLines` and `sensitivePaths` for its repositories and adds a test command.

## Repo profiles

Settings → Repo profiles (stored in the database, not `config.json`; `GET/POST /api/v1/profiles`, `PUT/DELETE /api/v1/profiles/{id}`, `GET /api/v1/profiles/export`, `POST /api/v1/profiles/import`). A profile applies to a repository found by its `origin` URL first (`github.com/acme/*`, any scheme, the longest pattern wins), then by its folder (`/work/acme`, `~/acme`). Every field but the name and "applies to" is optional; empty means the global setting.

| Field | Effect |
|---|---|
| `baseBranch` | Where new jobs start when the request names no base |
| `autonomy` | The most autonomy a job in these repositories may have (a ceiling, not a default: a looser request or agent value is lowered, and the job says so; the job page cannot raise it either, `above_profile`) |
| `permissionMode` | The most permissive tool permissions allowed here, the same way |
| `runtime` | Runtime plugin of new jobs (`opencode`, `codex`…) |
| `models` | Runtime plugin → model of new jobs; replaces that runtime's `runtimeModels` entry |
| `sensitivePaths`, `maxDiffLines` | Replace the global values; an empty list switches the sensitive-path hold off |
| `agentTokenBudget` | Replaces the global per-job budget (0 = off); the daily budget stays global |
| `testCommand` | One shell command (one line) run in the worktree at each commit proposal; if it fails or times out, the commit is held for a human (only the exit status is shown) |
| `lintCommand` | Like `testCommand`, run before it (a linter or formatter check); leave both empty to run nothing |
| `instructions` | Added to the job's `instructions.md` |
| `stagingBranch` | Stored; no flow uses it yet |

Changes apply to running jobs at once, except `instructions`, which is written at a job's creation and when its autonomy changes. Deleting a profile sends its jobs back to the global settings.

## What the UI can change

| Settings tab | Keys |
|---|---|
| General | `maxConcurrentAgents`, `pauseTimeoutSec`, `workspace`, `eventRetentionDays` |
| Gates & autonomy | `idleAlertMinutes`, `gateParkMinutes`, `freshSessions`, `autonomy`, `maxAutoApprovalsInRow`, `defaultRuntime`, `permissionMode`, `suggestAgents` |
| Notifications | `mutedNotifications`, `desktopNotifications`, `ntfyServer`, `ntfyTopic` |
| Open in | `openIn` |
| Git access | `gitUserName`, `gitUserEmail`, `gitIdentities`, `gitHosts` and their tokens |
| Repo profiles | Profiles (in the database) |
| Plugins | `disabledPlugins`, `pluginSettings`, plugin secrets, `toolchainDefaults`, `runtimeModels` (a runtime's Options) |

The other keys (`maxSessionMinutes`, `prPollMinutes`, `maxReviewLoops`, the budgets, `maxDiffLines`, `sensitivePaths`) have no form yet. `PUT /api/v1/config` sets them with range checks and they apply at once:

```sh
curl -X PUT http://127.0.0.1:4740/api/v1/config \
  -H "Authorization: Bearer $(jq -r .userToken "$XHERD_HOME/config.json")" \
  -H 'Content-Type: application/json' -d '{"maxSessionMinutes": 90}'
```

Or edit `config.json` by hand and restart the daemon. Per repository, a repo profile sets `sensitivePaths`, `maxDiffLines` and the per-job budget from the UI.

## Per-job files

Every worktree has a `.xherd/` folder (excluded from git through the repository's `info/exclude`):

| Path | Contents |
|---|---|
| `job.json` | The job's identity and settings |
| `instructions.md` | What the job is told about xherd and its autonomy level |
| `spec/` | The documents of the flow (`spec.md`, `design.md`, `tasks.md`, `commits.md`, `review.md`, `pr.md`, …) and `memory.md` |
| `ledger/approvals.jsonl` | Every approval with the content hash. The file wins over the database on conflict |
| `toolchain.json` | The tools the worktree's version files ask for |
| `hooks/` | The worktree's git hooks, including the pre-push guard |
| `commit-error.txt` | Written when git refuses an approved commit |
| `HANDOFF.md` | The note a job writes when paused; read again on resume |
| `ledger/issues.jsonl` | Out-of-scope problems the job recorded (`xherd issue add`) |
| `AGENTS.proposed.md` | A proposed update of the repository's `AGENTS.md` (Shared memory) |
