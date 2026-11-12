# HTTP API

The daemon serves the web UI, the CLI and the agents over one HTTP API at `http://127.0.0.1:4740/api/v1` (the port is `config.port`). The web UI uses nothing else, so everything it does can be scripted. The API follows the daemon's version: there is no stability promise across releases yet, apart from the agent protocol ([XAP](xap.md), versioned on its own).

JSON on the wire is camelCase; enum values are kebab-case. Errors are `{"error": "<message>", "code": "<stable code>"}` with a 4xx or 5xx status. XAP calls are the exception: they answer `{ok: false, code, message}` (see [XAP §8](xap.md#8-http)).

## Authentication

There are two kinds of caller.

**The local user** has full rights. A request counts as the local user when it carries `Authorization: Bearer <userToken>` (the token in `config.json`, which the CLI sends), or when it comes from this machine with no token:

- the peer is loopback (natively), or the gateway the published port arrives through (in the container, `XHERD_LOCAL_PEERS`; loopback inside the container is where the agents are, so it does not count there),
- the `Host` is `localhost`, `127.0.0.1` or `[::1]`,
- no proxy headers (`Forwarded`, `X-Forwarded-*`, `X-Real-IP`),
- and `Origin` is absent or a local `http://` origin. A request from another site is refused, websockets included.

A token the daemon never issued adds nothing to a local request; a revoked one is refused.

**Agents** get a token at creation (`XHERD_AGENT_TOKEN` in their session) bound to that agent, with the scopes `hooks`, `xap` and `credential`. It works on `/hooks`, `/xap` and `/git/credential` only. No agent token can approve anything.

`GET /healthz` needs no authentication.

## Routes

All paths are under `/api/v1`. `{id}` is an agent id or name.

### Daemon and state

| Method and path | Does |
|---|---|
| `GET /healthz` | Liveness, no auth |
| `GET /state` | Everything the UI starts from: agents, flows (each pack flow's stages), plugins (with `enabled`), tracker links, `buildVersion` |
| `GET /ws` (WebSocket) | The change feed: one JSON message per change, tagged `type` (kebab-case). The UI reloads `/state` when it reconnects. A client that falls behind is disconnected |
| `GET /checks`, `POST /checks/{id}/fix` | Setup checks, and running a check's fix |
| `POST /install` | Install the pack's commands for the runtimes (`xherd install`) |
| `GET /security` | The `xherd security check` findings |
| `GET /config`, `PUT /config` | The saved and running configuration (`restartNeeded`), and a partial update with range checks; unknown keys are refused. See [Configuration](configuration.md) |

### Agents

| Method and path | Does |
|---|---|
| `GET /agents`, `POST /agents` | List, create (body: `ticketKey` and `repo` (a path or URL), optional `slug`, `flow`, `base`, `autonomy`, `permissionMode`, `runtime`, `prompt`; the same as `xherd new`) |
| `GET /agents/{id}` | One agent |
| `POST /agents/{id}/{action}` | `pause`, `resume`, `cancel`, `send {text}`, `approve {reason?, expect?, message?}`, `request-changes {comment}`, `autonomy {level}`, `mode {mode}`, `memory-accept {text?}`, `memory-dismiss`. `expect` maps each document to the hash the client showed (from `GET /file`); a mismatch is refused with `changed_since_read`. `message` replaces a commit's message |
| `POST /agents/pause-all` | Kill switch: pause every queued and live agent now, no handoff |
| `GET /agents/{id}/diff` | What is being approved: the commit proposal against `HEAD`, the branch at a ship gate, else every uncommitted change |
| `GET /agents/{id}/docs` | The spec folder's Markdown files, gate documents first |
| `GET /agents/{id}/file?path=` | One file: text (images as `dataUrl`), its document hash, and the last approved text |
| `GET /agents/{id}/findings` | `review.md` findings, with `resolved` |
| `GET /agents/{id}/viewed`, `POST /agents/{id}/viewed {path, fingerprint?}` | The Changes panel's "Viewed" marks (no fingerprint clears one) |
| `GET /agents/{id}/memory` | A shared-memory proposal: proposed text, current `AGENTS.md`, the `CLAUDE.md` an accept would write |
| `GET /agents/{id}/pr-review`, `PUT /agents/{id}/pr-decisions` | The PR comments of the current round with the agent's decisions, and your overrides |
| `GET /agents/{id}/ticket`, `GET /agents/{id}/ticket-status`, `GET /agents/{id}/ticket-summary`, `POST /agents/{id}/ticket-comment {body}` | Read the agent's ticket, whether a summary was posted, a drafted summary, and posting your text |
| `GET /agents/{id}/git-access` | The git doctor's first failure for the agent's host, for a `git access` block |
| `GET /agents/{id}/open-targets`, `POST /agents/{id}/open {app}` | The Open in menu, and opening one entry |
| `POST /agents/{id}/user-time {ms}` | Time you spent on the agent page (Time page) |

### Repositories and profiles

| Method and path | Does |
|---|---|
| `GET /repos` | Repositories used before and cloned ones, newest first |
| `POST /repos/resolve` | A path or URL → branches, default base, matching profile (or `suggestedMatch`) |
| `GET /profiles`, `POST /profiles`, `PUT /profiles/{id}`, `DELETE /profiles/{id}` | Repo profiles |
| `GET /profiles/export`, `POST /profiles/import` | All profiles as JSON; import is all or nothing and replaces profiles with the same name |

### Reviews, TODOs, time, backups

| Method and path | Does |
|---|---|
| `GET /reviews`, `POST /reviews/seen {items}` | Automatic approvals to look at, and marking them seen |
| `GET /todos`, `POST /todos`, `PUT /todos/{id}`, `DELETE /todos/{id}` | TODOs |
| `POST /todos/{id}/complete`, `POST /todos/{id}/reschedule {due}`, `POST /todos/{id}/check` | Complete, reschedule, run a condition check now |
| `GET /worklog?days=N` | Agent and user time per day and ticket, with delivery metrics |
| `GET /backups`, `POST /backups` | List and create backups |

### Git access, plugins, toolchains, Open in

| Method and path | Does |
|---|---|
| `GET /git/hosts`, `PUT /git/hosts` | Hosts, their provider and credential source (`providers` lists the host plugins) |
| `POST /git/doctor`, `POST /git/known-hosts`, `POST /git/login` | Check access to a host, accept a host key (re-scanned, same fingerprint required), start a browser login in a terminal |
| `POST /git/credential` | The git credential helper's endpoint (local user or an agent token with `credential`) |
| `POST /plugins/{name}/login` | Start a plugin's login in the setup terminal |
| `GET /plugins/{name}/secrets`, `PUT /plugins/{name}/secrets` | Which declared secrets are set (never their values), and setting them |
| `GET /toolchains`, `GET /toolchains/installed`, `GET /toolchains/{tool}/versions`, `POST /toolchains/install` | Toolchain tools, installed versions, installable versions (container), installing one |
| `GET /open-in` | Settings → Open in: links, the companion's apps and terminals |
| `GET /companion/next` | Long poll used by `xherd companion` |

### Terminals (WebSocket)

| Path | Does |
|---|---|
| `GET /ws/term/{id}?cols=&rows=` | An agent's tmux session. Binary frames carry terminal bytes both ways; a text frame `{"type": "resize", "cols", "rows"}` resizes |
| `GET /ws/setup/{id}` | The setup terminals: `git-login`, `plugin-login` (plugin logins and installs) |

### Agents and runtimes

| Method and path | Does |
|---|---|
| `POST /hooks` | A runtime hook or `xherd report` (scope `hooks`). Reports carry a sequence number; an older one is ignored |
| `POST /xap` | An XAP call (scope `xap`). See [XAP](xap.md) |
