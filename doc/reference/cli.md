# CLI reference

`xherd` is one binary: the daemon, the user commands, the calls jobs make, and a few helpers. Run `xherd <command> --help` for the exact flags of your version, and `xherd --version` for the version. The global `--json` flag gives machine-readable output for the XAP calls and `xherd diff`.

The CLI talks to the daemon over [HTTP](http-api.md) at `http://127.0.0.1:<port>` using the user token in the data root's `config.json`. With Docker, run it inside the container as the daemon's user: `docker exec -u xherd-daemon xherd xherd <command>` (see [Docker](../user/docker.md#two-users-inside-the-container)).

## Daemon and setup

| Command | Does |
|---|---|
| `xherd daemon` | Run the daemon in the foreground. Logs are JSON on stderr; `XHERD_LOG=debug` for more |
| `xherd daemon restart` | Ask the running daemon to restart (`POST /admin/restart`) and wait up to 60 s until it is down and answers `/healthz` again. The daemon closes its connections (10 s grace) and replaces itself with the same binary and arguments, so new plugins load while agent sessions in tmux keep running |
| `xherd status` | Is the daemon up? |
| `xherd plugin ls` | Plugins with their kind and version; an installed one shows `url@short-sha`, one copied by hand shows `installed by hand, not pinned`. Adds a note when a restart is needed |
| `xherd plugin install URL [--ref REF] [--path DIR] [--yes]` | Fetch a git repository (any remote URL; `http://` is refused), preview what it holds (name, version, where it comes from, what it will run, a warning, whether it replaces another plugin, a digest), ask, then install exactly the previewed digest. A repository with several plugins prints the choices and exits 2 asking for `--path`. Private repositories use [Git access](../user/guide.md#welcome-plugins-and-git-access). Needs a restart to load |
| `xherd plugin update NAME [--ref REF] [--yes]` | Same for an installed plugin: also prints the manifest diff, then installs the previewed digest. A plugin without a recorded source cannot be updated |
| `xherd plugin remove NAME [--forget] [--yes]` | Delete an external plugin's folder; `--forget` also drops its settings and secrets. Refused while a live job uses it |
| `xherd install [--runtime R] [--pack P]` | Install the pack's commands (`/xherd:*`) for runtimes. Default: the xherd pack, every runtime plugin that is switched on. Runs in the daemon, so commands land where jobs run |
| `xherd docker init [--workspace DIR] [--image I] [--port N] [--extra-path DIR]… [--force]` | Write `~/.config/xherd/compose.yaml` and `.env` for the detected engine. `--extra-path` mounts another folder with repositories at the same path; `--force` rewrites an existing file |
| `xherd docker doctor` | Check engine, mounts, ownership, SSH job and port binding; each finding has a fix |
| `xherd docker update [--no-pull]` | Pause live jobs, take a backup (the last 3 of these are kept; a failed backup stops the update unless `--force`), pull and recreate the container, wait for health, resume |
| `xherd security check` | Exposure overview: network binding, user split, plugins, limits, autonomy. Informational: warnings do not change the exit code |
| `xherd backup create [--to DIR]` | One `tar.zst`: database (jobs, saved agents with versions and memories), config (no user token), plugins, `skills/`, `archive/`, and for each unfinished job its `.xherd/` notes, a git bundle of its commits beyond the base, a patch of uncommitted changes and its untracked files. No secrets, logins, toolchains or clones. Default folder: `<data>/backups`; in the container `<workspace>/xherd-backups` |
| `xherd backup verify ARCHIVE` | Check hashes, the database and the bundles without restoring |
| `xherd backup restore ARCHIVE [--workspace DIR] [--map OLD=NEW]… [--force]` | With the daemon stopped. Jobs come back `paused`. Missing worktrees are recreated (the repository is cloned from its remote when absent); existing ones are only completed. `--workspace` is the new machine's workspace; `--map` rewrites other repository paths. An existing database needs `--force` (kept in `backups/`). Git tokens and logins are entered again |
| `xherd git doctor [host] [--repo URL]` | Check SSH and HTTPS access to a host, in order, with the fix for the first failure |
| `xherd credential get` | git credential helper that answers with the token of Settings → Git access. Silent on any failure |

`xherd init` and `xherd runner` are used by the container's entrypoint; you do not run them yourself.

## Working with jobs

| Command | Does |
|---|---|
| `xherd new <NAME> [PROMPT]` | Create a job: worktree, branch and `.xherd/`. The name is lowercase letters, digits and `-` (up to 41), unique; it is the tmux session and the worktree folder |
| `xherd ls` | List jobs, with the saved agent they started from (`-` for none), the open gate or the PR URL (session ids cut to 8 characters) |
| `xherd diff <job>` | The open commit proposal's diff; at a ship gate the branch as it will be pushed; otherwise every uncommitted change |
| `xherd approve <job> [--reason TEXT]` | Approve the open gate (documents or commit) as they are on disk now; the reason goes to the ledger |
| `xherd changes <job> "<comment>"` | Ask the job to revise its open gate |
| `xherd send <job> "<text>"` | Type into the session; the answer to a question gate |
| `xherd pause --all` | Kill switch: pause every queued and live job now, with no handoff note; nothing new starts |
| `xherd pause\|resume\|cancel <job>` | Lifecycle. Pause asks the job for a handoff note first (up to `pauseTimeoutSec`), then ends the session; cancel ends it at once. Both keep the worktree, unless `cancel --remove-worktree` |
| `xherd remove-worktree <job>` | Delete a `done` or `cancelled` job's worktree folder, with its uncommitted changes and `.xherd/` documents; the branch and the job's record stay, and the coding agent can no longer be resumed |
| `xherd talk <job> "<text>"` | Ask a paused, parked, watching or failed job something: starts a session with the text as its first prompt and keeps it open (no auto-approval, no move to watching) until `xherd finish` or until the coding agent does flow work. (`xherd ask` is the jobs' question call) |
| `xherd finish <job>` | End a `talk` session: back to watching its PR when it was watching, else paused |
| `xherd done <job>` | Mark a job with no workflow done (it has no PR to merge): from `idle`, `waiting-input`, `paused` or `parked`; the session ends and, when the job has no saved agent, xherd offers to save its setup as one |
| `xherd resolve-conflicts <job>` | For a job with a PR that conflicts with its base: a new session merges the base branch (`origin/<base>`) into the branch and resolves the conflicts; the merge commit is proposed and held for your approval, then the coding agent pushes. Only a `watching`, `paused`, `error` or `idle` job with no open gate |
| `xherd autonomy <job> <ask\|important>` | Change what xherd may approve by itself (rewrites the job's instructions and tries an auto-approval at once) |
| `xherd mode <job> <ask\|edits\|auto\|all>` | Change the job's tool permissions; the running session keeps its mode, the next session start uses the new one. `all` only in the container |
| `xherd model <job> <model>` | Change the job's model (`""` = the runtime's default); like `mode`, it applies from the next session start |
| `xherd attach <job>` | Attach this terminal to the job's tmux session (in the container when using Docker) |
| `xherd shell <job>` | A shell in the job's worktree |
| `xherd open <job> [--app A]` | Open the worktree in an app on this computer: apps from `~/.config/xherd/open.json`, else editors found on `PATH` |
| `xherd terminal-handler install\|uninstall` | Register (or remove) the `xherd-term://` link behind Open in → Terminal, for the user running it: the handler `xherd-open-terminal` opens your own terminal app in a job's worktree, and accepts nothing but a job worktree under this daemon's worktrees folder. Linux: `~/.local/bin/xherd-open-terminal` and `xherd-term.desktop` (`xdg-mime`); macOS: a small app in `~/Library/Application Support/xherd/` (not checked on a Mac yet). Then it tells the daemon, which shows the Terminal entry; with the daemon down it warns and still installs. Native installs only: in Docker run the `curl … \| sh` command from Settings → Open in on your computer (refused inside the container). Settings → Open in → Install does the same |

`xherd new` options:

| Flag | Meaning |
|---|---|
| `--ticket` | The ticket: an issue URL (`https://acme.atlassian.net/browse/PAY-412`, `https://github.com/acme/api/issues/12`) or a key (`PAY-412`, `#12`). Its key starts the branch name (`PAY-412-<name>`, `12-<name>`); without one the branch is the name. A URL no enabled tracker reads is kept as a link only |
| `--flow` | A flow of the built-in pack (`feature` or `bugfix`), or `none` for a plain session with your prompt (default `feature`) |
| `--repo` | A local path, or a URL cloned once into `<workspace>/repos/<host>/<path>`. Default: the current directory when it is inside a git repository; otherwise, with an `https://` `--ticket` that an enabled tracker reads, the ticket's repository (`https://github.com/acme/api/issues/12` → `https://github.com/acme/api`), cloned like any URL. Without either, `repo_required`. An explicit `--repo` always wins |
| `--base` | Base branch (default: the repository's default) |
| `--autonomy` | `ask` (default) or `important` |
| `--mode` | Tool permissions: `ask`, `edits`, `auto` or `all` (container only); default: the repo profile's, else the `permissionMode` setting |
| `--runtime` | Runtime plugin: `claude-code`, `opencode` or `codex`; default: the repo profile's, else the `defaultRuntime` setting |
| `--model` | Model for the runtime (`opus`, `sonnet`, `opencode/…`); default: the repo profile's `models` entry for the runtime, else the `runtimeModels` setting, else the runtime's own |
| `--agent` | Start from a saved agent (id or name, see below), for every stage no [assignment](#workflows-and-agents) covers: its settings are the defaults for the flags above, and a flag you give wins. The agent has no repository: `--repo`, the ticket or the repo profile give it |
| `--stage-agent STAGE=AGENT` | Repeat to give one stage of this job to another agent than the workflow does (`--stage-agent design=architect`). Refusals `unknown_stage`, `unknown_agent`, `agent_archived`, `mixed_runtime` |

## Saved agents

A saved agent is a named, versioned setup (runtime, model, autonomy, tool permissions, instructions, limits; no repository, base branch or workflow) that jobs start from. Every `xherd agent` command takes the agent's name or id; `--json` prints the API's answer.

| Command | Does |
|---|---|
| `xherd agent ls` | Agents with their version, status, runtime, workflow, live jobs and last job |
| `xherd agent show <agent>` | Every setting of one agent |
| `xherd agent new <name> [flags]` | Save an agent. Default runtime `claude-code` |
| `xherd agent edit <agent> [flags]` | Change the settings given; the agent's version goes up by one. The name cannot change |
| `xherd agent archive\|unarchive <agent>` | Hide an agent from pickers, or bring it back (same version) |
| `xherd agent rm <agent>` | Delete an agent with no live jobs (its jobs keep their snapshot) |
| `xherd agent jobs <agent>` | The jobs started from it |
| `xherd agent start <agent> [PROMPT] [--ticket T] [--repo R] [--base B] [--name N]` | Start a job with the agent on every stage. Without `--name` it is `<agent>-<ticket key lower-cased>`, else `<agent>-<n>` |
| `xherd agent export [<agent>] [--out FILE]` | The agents as JSON (no ids, no secrets) |
| `xherd agent import FILE [--yes]` | Prints every setting of each agent in the file first, with `!` and a "look at" line for the ones that widen what a job may do or add text it reads (`permissionMode` `auto`/`all`, `autonomy` other than `ask`, `instructions`, `skills`), then asks (`--yes` skips the question). Fields an agent no longer has (`repo`, `baseBranch`, `workflow`, `stageOverrides`) are listed as ignored. All or nothing; a name that exists is refused, never replaced |

`new` and `edit` flags: `--description`, `--runtime`, `--fallback-runtime` (used once, at creation, when the runtime is unusable), `--model` (for the agent's runtime), `--autonomy`, `--mode`, `--instructions` or `--instructions-file`, `--job-token-budget`, `--sensitive-path` (repeat), `--max-diff-lines`, `--skill` (repeat), `--max-live-jobs`. On `edit` a repeated flag replaces its whole list, and an empty value (`--model ""`, `--sensitive-path ""`) clears the setting.

## Workflows and agents

A workflow's stages can be given to saved agents (an architect for the spec and design, a coder for the implementation, a reviewer for the review). `<flow>` is `feature`, or `pack/flow` for a flow of another pack.

| Command | Does |
|---|---|
| `xherd flow ls` | Every flow with how many of its stages have an agent |
| `xherd flow show <flow>` | Each stage with the agent it resolves to: `(same)` when it follows the stage before, `human`, and why an agent cannot run |
| `xherd flow assign <flow> <stage> <agent> [--human\|--no-human]` | Give a stage to an agent, replacing its assignment. `--human` makes the stage's gate always wait for you, whatever the autonomy |
| `xherd flow unassign <flow> <stage>` | Take the stage back: it uses the agent given at creation, else the stage before |

Refusals: `unknown_pack`, `unknown_flow`, `unknown_stage`, `unknown_agent`, `agent_archived`, `mixed_runtime` (all agents of a flow share one runtime). Assignments apply to jobs created afterwards.

## Called by the runtime and by jobs

These are not for daily use; they are what Claude Code and the jobs call.

| Command | Does |
|---|---|
| `xherd hook <event>` | Runtime hook; reads the hook JSON on stdin. Silent, always exits 0 |
| `xherd report …` | Push status for jobs or wrappers without hooks |
| `xherd stage enter`, `gate open`, `ask`, `tasks sync`, `task start\|done\|block`, `commit propose\|done\|drop`, `pr opened`, `handoff written`, `issue add`, `memory propose`, `agent memory propose`, `agent propose` | The agent protocol, XAP. See the [XAP reference](xap.md) |
| `xherd toolchain exec -- <argv>` | Provide the tools `.xherd/toolchain.json` lists, then run the command with them on `PATH`. The daemon starts jobs this way |
| `xherd guard pre-push --branch B` | The check behind the worktree's `pre-push` hook: only branch `B` may be pushed |

Exit codes of XAP calls: `0` accepted, `2` refused (stderr has a one-line reason), `3` daemon unreachable.

## Environment variables

| Variable | Used by | Meaning |
|---|---|---|
| `XHERD_HOME` | everything | Data root. Else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd` |
| `XHERD_PLUGIN_INSTALL=off` | daemon | Turn plugin install and update off (CLI, API and UI); removing stays possible |
| `XHERD_WORKSPACE` | daemon | Workspace folder when `config.workspace` is not set (default `~/xherd`) |
| `XHERD_URL` | CLI, jobs | Daemon URL; set in job sessions, read by the CLI when present |
| `XHERD_LOG` | daemon | Log filter, e.g. `debug` |
| `XHERD_ENV=1` | jobs | Set in job sessions; SDKs call xherd only when it is `1` |
| `XHERD_BIN`, `XHERD_JOB_TOKEN`, `XHERD_PORT_BASE` | jobs | The binary path, the job's token, the first of its 10 reserved ports |
| `XHERD_SANDBOX=1`, `XHERD_BIND`, `XHERD_LOCAL_PEERS`, `XHERD_QUEUE` | container | See [Docker](../user/docker.md) |
| `XHERD_UID`, `XHERD_GID`, `XHERD_PORT`, `XHERD_IMAGE`, `XHERD_SSH_SOCK` | compose `.env` | Your ids, published port, image to run, forwarded SSH job socket (written by `xherd docker init`) |
| `XHERD_EXTRA_PATHS` | container | Colon-separated extra folders mounted at the same path where repositories may live outside the workspace |
