# CLI reference

`xherd` is one binary: the daemon, the user commands, the calls agents make, and a few helpers. Run `xherd <command> --help` for the exact flags of your version. Every command accepts `--json` for machine-readable output.

The CLI talks to the daemon over HTTP at `http://127.0.0.1:<port>` using the user token in the data root's `config.json`. With Docker, run it inside the container as the daemon's user: `docker exec -u xherd-daemon xherd xherd <command>` (see [Docker](../user/docker.md#two-users-inside-the-container)).

## Daemon and setup

| Command | Does |
|---|---|
| `xherd daemon` | Run the daemon in the foreground. Logs are JSON on stderr; `XHERD_LOG=debug` for more |
| `xherd status` | Is the daemon up? |
| `xherd install [--runtime R] [--pack P]` | Install the pack's commands (`/xherd:*`) for agent runtimes. Default: the xherd pack, every runtime plugin that is switched on. Runs in the daemon, so commands land where agents run |
| `xherd docker init` | Write `~/.config/xherd/compose.yaml` and `.env` for the detected engine |
| `xherd docker doctor` | Check engine, mounts, ownership, SSH agent and port binding; each finding has a fix |
| `xherd docker update [--no-pull]` | Pause live agents, take a backup (the last 3 of these are kept; a failed backup stops the update unless `--force`), pull and recreate the container, wait for health, resume |
| `xherd security check` | Exposure overview: network binding, user split, plugins, limits, autonomy. Informational: warnings do not change the exit code |
| `xherd backup create [--to DIR]` | One `tar.zst`: database, config (no user token), plugins, `archive/`, and for each unfinished agent its `.xherd/` notes, a git bundle of its commits beyond the base, a patch of uncommitted changes and its untracked files. No secrets, logins, toolchains or clones. Default folder: `<data>/backups`; in the container `<workspace>/xherd-backups` |
| `xherd backup verify ARCHIVE` | Check hashes, the database and the bundles without restoring |
| `xherd backup restore ARCHIVE [--workspace DIR] [--map OLD=NEW]… [--force]` | With the daemon stopped. Agents come back `paused`. Missing worktrees are recreated (the repository is cloned from its remote when absent); existing ones are only completed. `--workspace` is the new machine's workspace; `--map` rewrites other repository paths. An existing database needs `--force` (kept in `backups/`). Git tokens and logins are entered again |
| `xherd git doctor [host] [--repo URL]` | Check SSH and HTTPS access to a host, in order, with the fix for the first failure |
| `xherd credential get` | git credential helper that answers with the token of Settings → Git access. Silent on any failure |

`xherd init` and `xherd runner` are used by the container's entrypoint; you do not run them yourself.

## Working with agents

| Command | Does |
|---|---|
| `xherd new <KEY> [PROMPT]` | Create an agent: worktree, branch and `.xherd/` for a ticket |
| `xherd ls` | List agents, with the open gate or the PR URL (session ids cut to 8 characters) |
| `xherd diff <agent>` | The open commit proposal's diff; at a ship gate the branch as it will be pushed; otherwise every uncommitted change |
| `xherd approve <agent>` | Approve the open gate (documents or commit) as they are on disk now |
| `xherd changes <agent> "<comment>"` | Ask the agent to revise its open gate |
| `xherd send <agent> "<text>"` | Type into the session; the answer to a question gate |
| `xherd pause --all` | Kill switch: pause every queued and live agent now, with no handoff note; nothing new starts |
| `xherd pause\|resume\|cancel <agent>` | Lifecycle. Pause asks the agent for a handoff note first (up to `pauseTimeoutSec`), then ends the session; cancel ends it at once. Both keep the worktree |
| `xherd autonomy <agent> <ask\|important>` | Change what xherd may approve by itself (rewrites the agent's instructions and tries an auto-approval at once) |
| `xherd mode <agent> <ask\|edits\|auto\|all>` | Change the agent's tool permissions; the running session keeps its mode, the next session start uses the new one. `all` only in the container |
| `xherd attach <agent>` | Attach this terminal to the agent's tmux session (in the container when using Docker) |
| `xherd shell <agent>` | A shell in the agent's worktree |
| `xherd open <agent> [--app A]` | Open the worktree in an app on this computer: apps from `~/.config/xherd/open.json`, else editors found on `PATH` |
| `xherd companion` | Run Open-in requests from the browser on this computer: the terminal (`xherd shell <agent>` in the chosen terminal, else this computer's default: on macOS Terminal, iTerm or Ghostty; on Linux the desktop's default (`$TERMINAL`, KDE's `kdeglobals`, GNOME's `gsettings`) among those installed, e.g. Ghostty, Konsole, GNOME Terminal, kitty), its apps, and your commands from Settings → Open in, each asked about here (`[y/N]`) the first time and after any change, then remembered in `~/.config/xherd/open-approved.json`. Keep it open in a terminal: without one it refuses commands it has not confirmed |

`xherd new` options:

| Flag | Meaning |
|---|---|
| `--slug` | Short name used in the branch and worktree |
| `--flow` | A flow of the built-in pack, or `none` for a plain session with your prompt (default `feature`) |
| `--repo` | A local path, or a URL cloned once into `<workspace>/repos/<host>/<path>`. Default: the current directory |
| `--base` | Base branch (default: the repository's default) |
| `--autonomy` | `ask` (default) or `important` |
| `--mode` | Tool permissions: `ask`, `edits`, `auto` or `all` (container only); default: the repo profile's, else the `permissionMode` setting |
| `--runtime` | Runtime plugin: `claude-code`, `opencode` or `codex`; default: the repo profile's, else the `defaultRuntime` setting |

## Called by the runtime and by agents

These are not for daily use; they are what Claude Code and the agents call.

| Command | Does |
|---|---|
| `xherd hook <event>` | Runtime hook; reads the hook JSON on stdin. Silent, always exits 0 |
| `xherd report …` | Push status for agents or wrappers without hooks |
| `xherd stage enter`, `gate open`, `ask`, `tasks sync`, `task start\|done\|block`, `commit propose\|done\|drop`, `pr opened`, `handoff written`, `issue add` | The agent protocol, XAP. See the [XAP reference](xap.md) |
| `xherd toolchain exec -- <argv>` | Provide the tools `.xherd/toolchain.json` lists, then run the command with them on `PATH`. The daemon starts agents this way |
| `xherd guard pre-push` | The check behind the worktree's `pre-push` hook |

Exit codes of XAP calls: `0` accepted, `2` refused (stderr has a one-line reason), `3` daemon unreachable.

## Environment variables

| Variable | Used by | Meaning |
|---|---|---|
| `XHERD_HOME` | everything | Data root. Else `$XDG_DATA_HOME/xherd`, else `~/.local/share/xherd` |
| `XHERD_WORKSPACE` | daemon | Workspace folder when `config.workspace` is not set (default `~/xherd`) |
| `XHERD_URL` | CLI, agents | Daemon URL; set in agent sessions, read by the CLI when present |
| `XHERD_LOG` | daemon | Log filter, e.g. `debug` |
| `XHERD_ENV=1` | agents | Set in agent sessions; SDKs call xherd only when it is `1` |
| `XHERD_BIN`, `XHERD_AGENT_TOKEN`, `XHERD_PORT_BASE` | agents | The binary path, the agent's token, the first of its 10 reserved ports |
| `XHERD_SANDBOX=1`, `XHERD_BIND`, `XHERD_LOCAL_PEERS`, `XHERD_QUEUE` | container | See [Docker](../user/docker.md) |
| `XHERD_UID`, `XHERD_GID`, `XHERD_PORT`, `XHERD_IMAGE`, `XHERD_SSH_SOCK` | compose `.env` | Your ids, published port, image to run, forwarded SSH agent socket (written by `xherd docker init`) |
| `XHERD_EXTRA_PATHS` | container | Colon-separated extra folders mounted at the same path where repositories may live outside the workspace |
