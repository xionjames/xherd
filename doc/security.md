# Security model

xherd runs coding agents that can read, write and execute. This page says what it protects, what it does not, and how to check your setup. `xherd security check` (and Settings → Setup checks) report the current state; it is informational and never changes an exit code.

## Threats it is built for

| Threat | Mechanism |
|---|---|
| An agent is wrong or overreaches | Approval gates: nothing moves on without a human approval (or an autonomy level you chose), and approvals are of exact content hashes |
| An agent touches files it should not | Each agent has its own worktree; a commit proposal is checked against the `files:` of the tasks it lists (scope fence) |
| A secret or a risky change slips into a commit | Tripwires on commit proposals: secret patterns, sensitive path globs, diff size. A hit holds the gate for a human |
| A bad push | Pre-push guard in every worktree |
| Text from a pull request or ticket steers the agent | Untrusted text is wrapped in a block with a per-message random token, and the instructions say not to follow it. A reply built from comments is only posted after a human-only approval |
| Runaway cost or time | Token budgets (per agent and per day), a session length limit, a run limit of consecutive auto-approvals, a review loop limit |
| An agent reads xherd's own secrets | In Docker: the daemon runs as a different user with its own volume (see below) |
| A web page attacks the local daemon | Requests carrying another site's `Origin` are refused; local rights come from the address, not from a cookie |

## What it does not do

- It is not a sandbox against a determined hostile agent running natively: the agents and the daemon share a user, so xherd's `0600` files are a guardrail, not a boundary. Use the Docker install for a boundary.
- The pre-push guard stops mistakes. `git push --no-verify` skips it; branch protection on the host is the real boundary.
- Agents can reach the network from the container. There is no egress filtering.
- Docker-in-Docker is not available, and the container shares one network between agents.
- It is a single-user tool. There is no sign-in, no roles and no remote access story yet; do not expose the port.

## Approval integrity

- A document is approved if and only if its current content hash has an entry in `.xherd/ledger/approvals.jsonl`. An edit changes the hash and re-opens the gate.
- The UI sends the hashes it displayed. If a file changed between display and click, the approval is refused (`changed_since_read`), so you approve exactly what you read.
- A commit approval covers exactly the proposed files; xherd runs the commit itself, hooks included, never with `--no-verify`.
- Automatic approvals are recorded with `auto` and listed under *To review*. A gate that xherd inferred (the agent stopped without declaring it) always waits for a human, and so do gates in `critical`/`human-only` classes, the ship stage of a hotfix, and the pull-request review stage.
- Agents cannot approve. Their tokens carry only the `hooks`, `xap` and `credential` scopes.

## Network and access

- The daemon binds `127.0.0.1` natively. In the container it binds `0.0.0.0` inside, and compose publishes the port on the host's `127.0.0.1` only; `xherd docker doctor` checks that.
- Local rights (the local browser, the CLI) come from the peer address plus a local or absent `Origin`. In the container loopback is where the agents are, so only the gateway the published port arrives from counts as local, and the CLI there needs its token.
- Changing `bind` to a non-loopback address makes the daemon reachable from the network. Requests from other machines do not get the local user's rights, but the daemon is not built to face a network. `xherd security check` warns about it.

## Two users in the container

The daemon runs as `xherd-daemon`; agents run as a user with your host ids. The daemon's `config.json`, `secrets.json` and database are in a volume only it can read. A small `xherd runner` process running as the agents' user does the work that must happen as that user (git, verify commands, plugin commands, tmux). Details in [Docker](docker.md#two-users-inside-the-container). File-permission enforcement between the two users needs a Linux host; Colima's mounts do not enforce ownership.

## Secrets

- Git tokens and plugin secrets are in `secrets.json` (mode `0600`) and reach git only through the credential helper, never in a URL or `.git/config`, and reach `curl` on stdin only.
- Output captured from commands is redacted before it is logged.
- Tripwire findings are masked to four characters, and messages never include the matched text.
- Do not paste secrets into prompts: agent transcripts are plain files in the runtime's folder.

## Plugins

Built-in plugins are embedded. External plugins in `<data root>/plugins/` run with your rights and are not pinned to a version; the security check lists them. Install only plugins you have read.

## Reporting a problem

There is no private reporting channel yet. Open a GitHub issue and leave out exploit details.
