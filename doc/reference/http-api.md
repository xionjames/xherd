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
| `GET /state` | Everything the UI starts from: agents, flows (each pack flow's stages), plugins (with `enabled`, and `view: {entry}` for a view plugin), tracker links, `buildVersion` |
| `GET /ws` (WebSocket) | The change feed: one JSON message per change, tagged `type` (kebab-case). The UI reloads `/state` when it reconnects. A client that falls behind is disconnected |
| `GET /checks`, `POST /checks/{id}/fix` | Setup checks, and running a check's fix |
| `POST /install` | Install the pack's commands for the runtimes (`xherd install`) |
| `GET /security` | The `xherd security check` findings |
| `GET /config`, `PUT /config` | The saved and running configuration (`restartNeeded`), and a partial update with range checks; unknown keys are refused. See [Configuration](configuration.md) |

### Agents

| Method and path | Does |
|---|---|
| `GET /runtimes/{name}/models` | A runtime's model suggestions: `{choose, models: [{id, label?, tier?}], error?}`; `choose` is false when the runtime takes no model, `error` says why its list command gave nothing |
| `GET /agents`, `POST /agents` | List, create (body: `name`, optional `repo` (a path or URL; when omitted or blank and `ticket` is an issue URL an enabled tracker reads, the ticket's repository is used, else `repo_required`; an explicit `repo` wins), optional `ticket` (issue URL or key), `flow`, `base`, `autonomy`, `permissionMode`, `runtime`, `model`, `prompt`; the same as `xherd new`. Until the next release `ticketKey` and `slug` are still taken when `name` is missing: the name is the slug, else the key lower-cased. Refusals include `name_required`, `bad_name`, `bad_ticket`, `name_taken`, `branch_exists`, `repo_required`). An agent has `name`, `branch` and, with a ticket, `ticket: {key?, url?}`; `askHold: {at, wasWatching}` while an `ask` session is held, `worktreeRemovedAt` once its worktree is removed, and `autoApprovePending: true` while its open gate is one autonomy will approve at the next stop (it does not wait for you) |
| `POST /tickets/resolve {ticket, repo?}` | What a ticket field stands for: `{key, url, tracker, repo}` (`tracker` null when no enabled tracker reads it; `repo` is the clone URL of the issue's repository for a URL a tracker reads, else null; `repo`, a local path, picks the tracker of its host for a key) |
| `GET /agents/{id}` | One agent |
| `POST /agents/{id}/{action}` | `pause`, `resume`, `cancel {removeWorktree?}` (the cancel is made first; when the removal then fails the answer is still OK and a notification says why), `remove-worktree` (a `done` or `cancelled` agent only, else `not_allowed`; deletes the worktree folder, keeps the branch and the record, event `worktree-removed`; again = no change; refused with `no_workspace` for an agent without a worktree, `git` when git fails, with its message), `ask {text}` (a `paused`, `parked`, `watching` or `error` agent: a session whose first prompt is the text, held open until `finish` or flow work; refused with `usage` for an empty text, `not_allowed` for another status, `prompt_pending` while a next command waits), `finish` (ends an `ask` session: back to `watching` when it was watching with a PR, else `paused`; `not_asked` without one), `resolve-conflicts` (a `watching`, `paused`, `error` or `idle` agent with a PR: queues a clean session with the pack's `conflicts` command, which merges `origin/<base>` into the branch; the merge commit is always held for a human; refused with `no_pr`, `worktree_removed`, `not_allowed` (a flow without that command, another status, an open gate, an unsafe base name), `prompt_pending`; event `user.resolve-conflicts`), `pr-check` (read the PR's state, checks, mergeability and reviewers now, at any status, instead of waiting for the next poll; the answer comes back on `/ws`; refused with `no_pr` (409) without a PR URL, an enabled Git provider for its host, or the worktree), `send {text}`, `approve {reason?, expect?, message?}`, `request-changes {comment}`, `autonomy {level}`, `mode {mode}`, `model {model}` (empty or null = the runtime's default; refused with `bad_model` for an odd name, `model_unavailable` when the runtime takes no model), `memory-accept {text?}`, `memory-dismiss`. `expect` maps each document to the hash the client showed (from `GET /file`); a mismatch is refused with `changed_since_read`. `message` replaces a commit's message. Once the worktree is removed, `resume`, `ask` and the routes that read it answer `worktree_removed`; the ticket routes keep working (they read the repository), and `pr-review` / `pr-decisions` answer `worktree_removed` too, so the UI hides the PR comments |
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
| `GET /agents/{id}/open-targets` | The Open in menu: links the browser opens (`{id, label, kind: "url", url}`), the Terminal link only while a terminal handler install is recorded, your links, minus `openIn.hidden`. `no_workspace` for an agent without a worktree |
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
| `GET /git/hosts`, `PUT /git/hosts` | Hosts, their provider and credential source (`providers` lists the host plugins). `GET` also lists the implicit rows of the Git providers that are on, each with `from` = the plugin's name; they are never saved, and `PUT` takes only the user's rows (a row carrying `from` is refused) |
| `POST /git/doctor`, `POST /git/known-hosts`, `POST /git/login` | Check access to a host, accept a host key (re-scanned, same fingerprint required), start a browser login in a terminal |
| `POST /git/credential` | The git credential helper's endpoint (local user or an agent token with `credential`) |
| `POST /plugins/{name}/login` | Start a plugin's login in the setup terminal |
| `GET /plugins/{name}/secrets`, `PUT /plugins/{name}/secrets` | Which declared secrets are set (never their values), and setting them |
| `GET /plugins/{name}/view/{path}` | A file of an enabled [Herd view plugin](../extend/plugins.md#herd-view-plugins) (404 for any other plugin, a hidden or `..` path, or a file outside its folder; 413 over 16 MiB). No authentication. Served with `Content-Security-Policy: sandbox allow-scripts; …` (sources limited to the plugin's files and the view kit, no `connect-src`), `X-Content-Type-Options: nosniff`, `Cache-Control: no-cache`, `Referrer-Policy: no-referrer`, `Access-Control-Allow-Origin: *` |
| `GET /view-kit/{file}` | The view kit: `xherd-view.js` (protocol v1 for views) and `xherd.css` (design tokens and component classes). No authentication; same headers |
| `GET /toolchains`, `GET /toolchains/installed`, `GET /toolchains/{tool}/versions`, `POST /toolchains/install` | Toolchain tools, installed versions, installable versions (container), installing one |
| `GET /open-in` | Settings → Open in: `{links, native, handler: {shipped, installed, updateAvailable, rootChanged}}`; `installed` is the recorded terminal handler install (`version`, `xherd`, `os`, `terminals` found there, default first, `root`, `at`) or `null` |
| `GET /open-in/terminal-handler.sh` | The terminal handler's installer (`text/plain`), with the worktrees folder and a report address `http://<Host>` baked in; run on your computer as `curl -fsSL <origin>/api/v1/open-in/terminal-handler.sh \| sh` (`\| sh -s uninstall` to remove). `bad_host` for an odd `Host` |
| `POST /open-in/terminal-handler` | What the installer reports: `{version, xherd, os, terminals, root}` after an install, `{installed: false}` after an uninstall; kept in `<data>/terminal-handler.json`. `bad_report` when a value is out of shape |
| `POST /open-in/terminal-handler/install {uninstall?}` | A native daemon runs the installer itself, as you; answers `{ok, output}` (the installer's last lines). `native_only` in the container |

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
