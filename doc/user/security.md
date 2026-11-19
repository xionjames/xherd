# Security model

xherd runs coding agents that can read, write and execute. This page says what it protects, what it does not, and how to check your setup. `xherd security check` (and Settings → Setup checks) report the current state; it is informational and never changes an exit code.

## Threats it is built for

| Threat | Mechanism |
|---|---|
| A job is wrong or overreaches | Approval gates: nothing moves on without a human approval (or an autonomy level you chose), and approvals are of exact content hashes |
| A job touches files it should not | Each job has its own worktree; a commit proposal is checked against the `files:` of the tasks it lists (scope fence) |
| A secret or a risky change slips into a commit | Tripwires on commit proposals: secret patterns, sensitive path globs, diff size, and a repo profile's lint and test commands. A hit holds the gate for a human |
| A bad push | Pre-push guard in every worktree |
| Text from a pull request or ticket steers the job | PR comments reach the job wrapped in a block with a per-message random token, and the instructions say not to follow it. A reply built from comments is only posted after a human-only approval. Ticket text is shown to you, never given to jobs, and the UI renders it without anything that loads by itself (no images, media or remote styles) |
| Runaway cost or time | Token budgets (per job and per day), a session length limit, a run limit of consecutive auto-approvals, a review loop limit |
| A job reads xherd's own secrets | In Docker: the daemon runs as a different user with its own volume (see below) |
| A web page attacks the local daemon | Requests carrying another site's `Origin` are refused; local rights come from the address, not from a cookie |

## What it does not do

- It is not a sandbox against a determined hostile job running natively: the jobs and the daemon share a user, so xherd's `0600` files are a guardrail, not a boundary. Use the Docker install for a boundary.
- The pre-push guard stops mistakes. `git push --no-verify` skips it; branch protection on the host is the real boundary.
- Jobs can reach the network from the container. There is no egress filtering.
- Docker-in-Docker is not available, and the container shares one network between jobs.
- It is a single-user tool. There is no sign-in, no roles and no remote access story yet; do not expose the port.

## Approval integrity

- A document is approved if and only if its current content hash has an entry in `.xherd/ledger/approvals.jsonl`. An edit changes the hash and re-opens the gate.
- The UI sends the hashes it displayed. If a file changed between display and click, the approval is refused (`changed_since_read`), so you approve exactly what you read.
- A commit approval covers exactly the proposed files; xherd runs the commit itself, hooks included, never with `--no-verify`.
- Automatic approvals are recorded with `auto` and listed under *Inbox → Updates*. A gate that xherd inferred (the job stopped without declaring it) always waits for a human, and so do gates in `critical`/`human-only` classes and the pull-request review stage.
- Jobs cannot approve. Their tokens carry only the `hooks`, `xap` and `credential` scopes.

## Runtime binaries

- In the container the coding-agent CLIs (Claude Code, opencode, Codex) are downloaded by the daemon, not baked into the image. Each plugin pins a version and a sha256 per architecture; a download whose digest differs is refused.
- The binaries live in the agents' home (`xherd-home`), which the agents' user can write, like the toolchains. A job could replace its runtime binary for every later job; the same user already runs every job's commands, so this adds no new reach, but treat the volume as the agents' own.

## Network and access

- The daemon binds `127.0.0.1` natively. In the container it binds `0.0.0.0` inside, and compose publishes the port on the host's `127.0.0.1` only; `xherd docker doctor` checks that.
- Local rights (the local browser, the CLI) come from the peer address plus a local or absent `Origin`. In the container loopback is where the jobs are, so only the gateway the published port arrives from counts as local, and the CLI there needs its token.
- Changing `bind` to a non-loopback address makes the daemon reachable from the network. Requests from other machines do not get the local user's rights, but the daemon is not built to face a network. `xherd security check` warns about it.
- Open in → Terminal uses the `xherd-term://` link handler you install on your computer. Any web page can fire that link; your browser asks first (unless you chose "Always allow" for a site), and the handler opens only a folder that is itself a job worktree under xherd's worktrees folder (no subfolder, no symlink leading elsewhere), runs nothing taken from the link, and starts a plain shell there. That shell's own start-up (a git prompt, `direnv`) then runs in a folder a job wrote, as when you `cd` there yourself. Nothing on your computer runs a command sent by the daemon: every Open-in entry is a link.

## Two users in the container

The daemon runs as `xherd-daemon`; jobs run as a user with your host ids. The daemon's `config.json`, `secrets.json` and database are in a volume only it can read. A small `xherd runner` process running as the agents' user does the work that must happen as that user (git, verify commands, plugin commands, tmux). Details in [Docker](docker.md#two-users-inside-the-container). File-permission enforcement between the two users needs a Linux host; Colima's mounts do not enforce ownership.

## Secrets

- Git tokens and plugin secrets are in `secrets.json` (mode `0600`) and reach git only through the credential helper, never in a URL or `.git/config`, and reach `curl` on stdin only.
- Output captured from commands is redacted before it is logged.
- Tripwire findings are masked to four characters, and messages never include the matched text.
- Do not paste secrets into prompts: job transcripts are plain files in the runtime's folder.

- A host plugin's token goes only to hosts you named: a host with a row in Settings → Git access, or a name without wildcards in the plugin's own host list. A remote or PR URL that the job controls cannot make xherd send a token to another server.
- Nothing is posted to a ticket or a pull request without your click: PR replies after you approve the `pr-review` gate, a ticket summary after *Post to the ticket*. xherd's own posts carry a hidden marker so it never answers itself.

## Plugins

Built-in plugins are embedded. External plugins in `<data root>/plugins/` are code that runs with the rights below; the security check lists them. A plugin installed from a git URL (Settings → Plugins, `xherd plugin install`) is recorded with its `url@sha` and the digest of its files, and an update shows the manifest diff first, but xherd does not review the code: the preview tells you what the manifest runs, and you decide. xherd installs exactly the digest you previewed, so a repository that changes between preview and install is refused. A plugin copied in by hand is not pinned. Install only plugins you have read; `XHERD_PLUGIN_INSTALL=off` turns installing and updating off. Only the local user can install or remove (a job's token cannot), and each install, update and removal is recorded in the event log.

- Natively they run with your rights.
- In the container they run as the agents' user, not the daemon's: they can use the jobs' `gh`/`glab` logins but cannot read `config.json`, `secrets.json` or the database. The daemon copies them at start to a folder that user can read (a restart picks up a new, updated or removed plugin). They receive only the secrets they declare.

## Reporting a problem

There is no private reporting channel yet. Open a GitHub issue and leave out exploit details.
