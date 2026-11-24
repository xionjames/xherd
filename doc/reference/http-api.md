# HTTP API

The daemon serves the web UI, the CLI and the jobs over one HTTP API at `http://127.0.0.1:4740/api/v1` (the port is `config.port`). The web UI uses nothing else, so everything it does can be scripted. The API follows the daemon's version: there is no stability promise across releases yet, apart from the agent protocol ([XAP](xap.md), versioned on its own).

JSON on the wire is camelCase; enum values are kebab-case. Errors are `{"error": "<message>", "code": "<stable code>"}` with a 4xx or 5xx status. XAP calls are the exception: they answer `{ok: false, code, message}` (see [XAP §8](xap.md#8-http)).

## Authentication

There are two kinds of caller.

**The local user** has full rights. A request counts as the local user when it carries `Authorization: Bearer <userToken>` (the token in `config.json`, which the CLI sends), or when it comes from this machine with no token:

- the peer is loopback (natively), or the gateway the published port arrives through (in the container, `XHERD_LOCAL_PEERS`; loopback inside the container is where the jobs are, so it does not count there),
- the `Host` is `localhost`, `127.0.0.1` or `[::1]`,
- no proxy headers (`Forwarded`, `X-Forwarded-*`, `X-Real-IP`),
- and `Origin` is absent or a local `http://` origin. A request from another site is refused, websockets included.

A token the daemon never issued adds nothing to a local request; a revoked one is refused.

**Jobs** get a token at creation (`XHERD_JOB_TOKEN` in their session) bound to that job, with the scopes `hooks`, `xap` and `credential`. It works on `/hooks`, `/xap` and `/git/credential` only. No job token can approve anything.

`GET /healthz` needs no authentication.

## Routes

All paths are under `/api/v1`. `{id}` is a job id or name.

### Daemon and state

| Method and path | Does |
|---|---|
| `GET /healthz` | Liveness, no auth. Returns `ok`, `version`, `buildVersion` and `startId`: a string fixed for each process start (it changes on every restart, re-exec included), so a client can tell a restarted daemon from one that never went down |
| `GET /state` | Everything the UI starts from: jobs, saved agents (`agents`), flows (each pack flow's stages), plugins (with `enabled`, and `view: {entry}` for a view plugin), tracker links, `buildVersion` |
| `GET /ws` (WebSocket) | The change feed: one JSON message per change, tagged `type` (kebab-case). The UI reloads `/state` when it reconnects. A client that falls behind is disconnected |
| `GET /checks`, `POST /checks/{id}/fix` | Setup checks, and running a check's fix. A check may carry `"installing": true` while the daemon installs a runtime's CLI (container only): it has no fix, and `fix` answers 409 `install_running`. A failed install leaves the last lines of its output in `detail`. The hub sends `{"type": "checks"}` when such an install starts or ends; the UI reloads `/checks` |
| `POST /install` | Body `{runtime?, pack?, removeObsolete?}`. Install the pack's commands for the runtimes (`xherd install`); `removeObsolete` deletes the files the pack no longer ships |
| `GET /install/review` | `{files: [{runtime, path, state, edited}]}`: the files waiting for a decision, `state` `edited` (you changed a file the pack has a new text for) or `obsolete` (the pack no longer ships it; `edited` says you changed it) |
| `POST /install/resolve` | Body `{choices: [{runtime, path, choice}]}`, `choice` one of `use-new`, `keep-mine` (edited files) or `remove`, `keep` (obsolete files). Answers `{checks}`. 409 `not_pending` when a choice does not match the review; nothing changes then |
| `GET /security` | The `xherd security check` findings |
| `GET /config`, `PUT /config` | The saved and running configuration (`restartNeeded`), and a partial update with range checks; unknown keys are refused. See [Configuration](configuration.md) |

### Jobs

| Method and path | Does |
|---|---|
| `GET /runtimes/{name}/models` | A runtime's model suggestions: `{choose, models: [{id, label?, tier?}], error?}`; `choose` is false when the runtime takes no model, `error` says why its list command gave nothing |
| `GET /jobs`, `POST /jobs` | List, create (body: `name`, optional `repo` (a path or URL; when omitted or blank it is the saved agent's, else the repository of an issue-URL `ticket` that an enabled tracker reads, else `repo_required`; an explicit `repo` wins), optional `newProject` (a project name, `""` = the job's name: creates `<workspace>/projects/<project>` as a new git repository, branch `main`, and uses it instead of `repo`; together with `repo` or `base` it is refused with `usage`, a bad name with `bad_name`, an existing folder with `path_exists`), optional `agent` (a saved agent, id or name: its settings are the defaults), `ticket` (issue URL or key), `flow`, `base`, `autonomy`, `permissionMode`, `runtime`, `model`, `prompt`; the same as `xherd new`. Until the next release `ticketKey` and `slug` are still taken when `name` is missing: the name is the slug, else the key lower-cased. Refusals include `name_required`, `bad_name`, `bad_ticket`, `name_taken`, `branch_exists`, `repo_required`). A job has `name`, `branch` and, with a ticket, `ticket: {key?, url?}`; `askHold: {at, wasWatching}` while an `ask` session is held, `worktreeRemovedAt` once its worktree is removed, `noOrigin: true` once its ship gate was approved with no `origin` remote (see `set-origin`, `finish-local`), and `autoApprovePending: true` while its open gate is one autonomy will approve at the next stop (it does not wait for you); in the container a runtime whose CLI is missing or still installing is refused with `runtime_missing` |
| `POST /tickets/resolve {ticket, repo?}` | What a ticket field stands for: `{key, url, tracker, repo}` (`tracker` null when no enabled tracker reads it; `repo` is the clone URL of the issue's repository for a URL a tracker reads, else null; `repo`, a local path, picks the tracker of its host for a key) |
| `GET /jobs/{id}` | One job |
| `POST /jobs/{id}/{action}` | `pause`, `resume`, `cancel {removeWorktree?}` (the cancel is made first; when the removal then fails the answer is still OK and a notification says why), `remove-worktree` (a `done` or `cancelled` job only, else `not_allowed`; deletes the worktree folder, keeps the branch and the record, event `worktree-removed`; again = no change; refused with `no_workspace` for a job without a worktree, `git` when git fails, with its message), `ask {text}` (a `paused`, `parked`, `watching` or `error` job: a session whose first prompt is the text, held open until `finish` or flow work; refused with `usage` for an empty text, `not_allowed` for another status, `prompt_pending` while a next command waits), `finish` (ends an `ask` session: back to `watching` when it was watching with a PR, else `paused`; `not_asked` without one), `resolve-conflicts` (a `watching`, `paused`, `error` or `idle` job with a PR: queues a clean session with the pack's `conflicts` command, which merges `origin/<base>` into the branch; the merge commit is always held for a human; refused with `no_pr`, `worktree_removed`, `not_allowed` (a flow without that command, another status, an open gate, an unsafe base name), `prompt_pending`; event `user.resolve-conflicts`), `pr-check` (read the PR's state, checks, mergeability and reviewers now, at any status, instead of waiting for the next poll; the answer comes back on `/ws`; refused with `no_pr` (409) without a PR URL, an enabled Git provider for its host, or the worktree), `send {text}`, `approve {reason?, expect?, message?}`, `request-changes {comment}`, `autonomy {level}` and `mode {mode}` (both refused with `above_profile` when they go past the repo profile's ceiling), `model {model}` (empty or null = the runtime's default; refused with `bad_model` for an odd name, `model_unavailable` when the runtime takes no model), `set-origin {url}` and `finish-local` (a job that stopped without a remote: `noOrigin: true` on the job, no gate open, `idle` or `paused`; else `not_stopped`. `set-origin` adds `origin`, pushes the base branch without force and queues the ship command again; refused with `bad_remote` (not a repository URL: https, http, ssh or scp-like `git@host:owner/repo`; local paths and `file://` are refused), `push_failed` (git's message; the remote is removed again), `no_workspace`, `git`. `finish-local` merges the branch into the base branch where it is checked out and marks the job `done`; refused with `base_not_checked_out`, `dirty`, `merge_failed` (the merge is aborted), `no_workspace`, `git`; both leave the job unchanged when refused), `memory-accept {text?}`, `memory-dismiss`, `mark-done` (a job with no workflow that is `idle`, `waiting-input`, `paused` or `parked`: the session ends and the job is `done`; `not_allowed` otherwise, event `user.mark-done`). `expect` maps each document to the hash the client showed (from `GET /file`); a mismatch is refused with `changed_since_read`. `message` replaces a commit's message. Once the worktree is removed, `resume`, `ask` and the routes that read it answer `worktree_removed`; the ticket routes keep working (they read the repository), and `pr-review` / `pr-decisions` answer `worktree_removed` too, so the UI hides the PR comments |
| `POST /jobs/pause-all` | Kill switch: pause every queued and live job now, no handoff |
| `GET /jobs/{id}/actions` | The job's actions (`{def, source, approved, last, succeeded}` each, `last` being the latest run: state, times, exit code), `portBase`, `saveOffer` (ids of succeeded job actions that are not on the repo profile) and `hasProfile`. User only |
| `POST /jobs/{id}/actions/{a}/run`, `…/restart` `{approve?}` | Start the action in its own tmux session (`xherd-<job>-act-<id>`); `approve: true` approves the command as it stands (Allow and run). `restart` stops it first. Refused with `action_not_approved`, `action_running`, `action_stopping`, `action_changed` (the command changed since the client read it), `not_allowed` (a status that cannot run actions); answers `{job}` |
| `POST /jobs/{id}/actions/{a}/stop` | Ctrl-C, then a kill after 5 s; `action_not_running` otherwise |
| `DELETE /jobs/{id}/actions/{a}` | Remove an action of the job (stops it first) |
| `GET /jobs/{id}/actions/{a}/log` | `{log, last}`: the saved output of the last end (the last 1 MiB), `null` while none exists |
| `POST /jobs/{id}/actions/{a}/save-to-profile`, `POST /jobs/{id}/actions/save-to-profile {ids}` | Write the action(s) to the job's repo profile; this approves the command |
| `POST /jobs/{id}/actions/card-dismiss` | Hide the end-of-job *Save actions* card for this job |
| `GET /jobs/{id}/diff` | What is being approved: the commit proposal against `HEAD`, the branch at a ship gate, else every uncommitted change |
| `GET /jobs/{id}/diff/source?path=&view=&commit=` | The whole new-side file of a diff, for expanding context around hunks: `{path, rev, text}`. The side follows `view` and `commit` as in `/diff` (the worktree, a recorded commit's sha, or `HEAD` at a ship gate); the caller never names a revision. Unknown query keys are refused; `bad_path` (400), `no_such_doc` (404), `too_large` (413, over 2 MiB) |
| `GET /jobs/{id}/docs` | The spec folder's Markdown files, gate documents first |
| `GET /jobs/{id}/file?path=` | One file: text (images as `dataUrl`), its document hash, and the last approved text |
| `POST /jobs/{id}/files` | Upload files to the job: one `multipart/form-data` request, every part named `files` and carrying a filename. Each file is stored in `<workdir>/.xherd/uploads/` (a name reduced to letters, digits, `.`, `_` and `-`, no leading `.`, at most 100 bytes, numbered `-2`, `-3`… when taken, never overwriting or following a symlink; owned by the agents' user in Docker) and the answer is `{paths}`: the absolute stored paths, in upload order. The request has no 5 MiB body limit; the limits are `uploadMaxBytes` (one file), `uploadMaxJobBytes` (the job's uploads folder) and `uploadMaxFiles` (its file count), see [Configuration](configuration.md). User only. Refused with `too_large` (413; nothing of that request is kept), `usage` (400: no part, or a part without a filename), `bad_path` (`.xherd/uploads` is not a folder), `no_workspace`, `worktree_removed`, `io` |
| `GET /jobs/{id}/files?path=` | Download one regular file under the job's workdir (`path` absolute, or relative to the workdir): `application/octet-stream` with `Content-Length`, `Content-Disposition: attachment` and `nosniff`. User only. `bad_path` (400) for an empty path or one that, once symlinks are resolved, lies outside the workdir; `no_such_file` (404) for a missing file or a non-regular one; `no_workspace`, `worktree_removed`. Not the same as `GET /jobs/{id}/file` (a JSON preview) |
| `GET /jobs/{id}/findings` | `review.md` findings, with `resolved` |
| `GET /jobs/{id}/viewed`, `POST /jobs/{id}/viewed {path, fingerprint?}` | The Changes panel's "Viewed" marks (no fingerprint clears one) |
| `GET /jobs/{id}/memory` | A shared-memory proposal: proposed text, current `AGENTS.md`, the `CLAUDE.md` an accept would write |
| `GET /jobs/{id}/agent-memory` | The job's agent-memory proposal (`proposal`, `proposed` text, `agent`, the agent's `current` memory and `version`); `proposal` is `null` when there is none |
| `POST /jobs/{id}/agent-memory-accept` | `{text?}`: the edit, else the job's file, becomes the agent's memory (version + 1) and the proposal is cleared; `no_proposal`, `no_agent`, `bad_memory`. `POST /jobs/{id}/agent-memory-dismiss` only clears it |
| `POST /jobs/{id}/suggestion/accept` | `{agent}` (an agent as in `POST /agents`, the form the user edited): creates the agent (version 1) and attaches the job to it, answering `{agent, job}`; the job keeps its own settings and gets the agent's snapshot. Refused with `no_suggestion` (the job has none), `has_agent`, or the agent's own refusals (`name_taken`, `bad_agent`…) |
| `POST /jobs/{id}/suggestion/dismiss` | Remember that the user does not want this job's suggestion (`job.suggestion.dismissed`); no change when there is none |
| `POST /jobs/{id}/save-as-agent` | `{agent}`: the same as accepting, for any job without an agent and with no suggestion needed; `has_agent` otherwise |
| `GET /jobs/{id}/pr-review`, `PUT /jobs/{id}/pr-decisions` | The PR comments of the current round with the job's decisions, and your overrides |
| `GET /jobs/{id}/ticket`, `GET /jobs/{id}/ticket-status`, `GET /jobs/{id}/ticket-summary`, `POST /jobs/{id}/ticket-comment {body}` | Read the job's ticket, whether a summary was posted, a drafted summary, and posting your text |
| `GET /jobs/{id}/git-access` | The git doctor's first failure for the job's host, for a `git access` block |
| `GET /jobs/{id}/open-targets` | The Open in menu: links the browser opens (`{id, label, kind: "url", url}`), the Terminal link only while a terminal handler install is recorded, your links, minus `openIn.hidden`. `no_workspace` for a job without a worktree |
| `POST /jobs/{id}/user-time {ms}` | Time you spent on the job page (Time page) |

### Repositories and profiles

| Method and path | Does |
|---|---|
| `GET /repos` | Repositories used before and cloned ones, newest first |
| `POST /repos/resolve` | A path or URL → branches, default base, matching profile (or `suggestedMatch`) |
| `GET /profiles`, `POST /profiles`, `PUT /profiles/{id}`, `DELETE /profiles/{id}` | Repo profiles |
| `GET /profiles/export`, `POST /profiles/import` | All profiles as JSON; import is all or nothing and replaces profiles with the same name |

### Saved agents

A saved agent is a named, versioned setup that jobs start from (runtime, models, autonomy, tool permissions, instructions, guardrails, `maxLiveJobs`; no repository, base branch or workflow, which belong to the job and to stage assignments below). `{id}` is an agent's id or name; a name that no agent has but a job does answers the old job route's `410` (below). A saved agent wins over a job of the same name.

| Method and path | Does |
|---|---|
| `GET /agents` | `{agents}`: each agent's settings plus the derived `status` (`archived`, `needs-you`, `working`, `idle`), `broken` (why jobs cannot start from it now: its pack, flow or runtime is gone or switched off; absent when fine), `liveJobs`, `lastJob` (the newest job's name) and `tokensToday` (the day's tokens of its jobs, UTC; recorded under the budget scope `agent:<id>`, shown but not enforced). `/state` carries the same list as `agents` |
| `POST /agents` | Create (body: `name`, `runtime`, and optional `description`, `models` (runtime → model), `fallbackRuntime`, `autonomy`, `permissionMode`, `instructions`, `jobTokenBudget`, `sensitivePaths`, `maxDiffLines`, `skills`, `maxLiveJobs` (default 1)). `id`, `version` and the dates are assigned. Refusals: `bad_name`, `bad_agent`, `name_taken`, `unknown_runtime`, `runtime_unusable` (the default pack has no commands for it), `bad_fallback`, `mode_unavailable` (`all` outside the container), `autonomy_unavailable`; `PUT` also `mixed_runtime` (the change would mix runtimes in a flow that assigns the agent) |
| `GET /agents/{id}`, `PUT /agents/{id}`, `DELETE /agents/{id}` | One agent; replace its settings (the version goes up by one; the name cannot change: `name_fixed`); delete it with its versions and memory (refused with `agent_busy` while a job started from it is live, `agent_assigned` while a workflow stage names it; jobs keep their snapshot) |
| `POST /agents/{id}/archive`, `POST /agents/{id}/unarchive` | Hide it from pickers, or bring it back. Not a new version. Archiving is refused with `agent_assigned` while a stage names it |
| `GET /agents/{id}/versions` | `{versions}`, newest first; every version is kept |
| `GET /agents/{id}/memory`, `PUT /agents/{id}/memory` | `{text, version}` (empty, version 0, before the first save); `PUT {text}` replaces it, at most 4 000 characters (`bad_memory` 422). Each job copies the memory when it is created (`job.agent.memory`); later edits do not reach started jobs |
| `GET /agents/{id}/jobs` | `{jobs}` started from it, newest first |
| `POST /agents/{id}/jobs` | Start a job from the agent; the body is the one of `POST /jobs` (`agent` is taken from the path) with a name that may be left out: it is then `<agent>-<ticket key>`, else `<agent>-<n>`. A job carries `agents` (a snapshot per agent id: name, version, runtime, `fellBack`, autonomy, mode, model, launch, instructions, memory, guardrails, skills), `stageAgents` (stage → `{agent, human}`), `defaultAgent`, `currentAgent` (the agent of the current stage) and `pinned` (an agent attached to a running job, never switched); editing an agent later leaves them as they are and, when a repo profile's ceiling lowered a value, `lowered: [{field, from, to, profile}]`. Refusals: `unknown_agent`, `agent_archived`, `agent_broken` (neither its runtime nor its fallback is usable), `repo_required` (no repository in the request, the ticket or the repo profile) |
| `GET /agents/export?name=` | `{agents}` without ids, versions or dates (all, or the one named) |
| `POST /agents/import?dryRun=1` | Body `{agents}`. All or nothing; a name that exists is refused with `name_taken` (never replaced). With `dryRun=1` nothing is written: `{agents: [{agent, risky, nameTaken}], nameTaken}`, where `risky` lists the fields to look at before importing (`permissionMode` auto or all, `autonomy` above ask, `instructions`, `skills`) and `ignored` the dropped fields the file still has (`repo`, `baseBranch`, `workflow`, `stageOverrides`). The real call answers `{added}` |

Every write publishes `{"type": "agents"}` on `/ws`; the UI refetches `/agents`.

### Workflows and stage assignments

Which saved agent runs which stage of a workflow (a pack's flow). A job created afterwards keeps a snapshot of the agents its stages resolve to: the request's overrides (`agents`: stage → agent in `POST /jobs`), the assignment, the job's default agent (`agent`), else the stage before.

| Method and path | Does |
|---|---|
| `GET /flows` | `{flows}`: every flow of every pack with its `stages` (`id`, `label`, `role`, `class`, `command`, `assigned`, `human`, `agentId`, `agent`, `broken`: why the agent cannot run, such as archived or its runtime off, `freshCheck`: the fresh-context check is on for the stage, and `freshCheckAllowed`: the stage can have it, true for a spec or design stage with a document gate) |
| `GET /flows/{pack}/{flow}` | One flow, the same shape |
| `GET /flows/{pack}/{flow}/assignments` | `{assignments: [{stage, agentId, human}]}` |
| `PUT /flows/{pack}/{flow}/assignments` | Replace the flow's assignments with `{assignments}` (agents by id or name; `human` defaults to false). The optional `freshCheck` (stage ids) replaces the stages with the fresh-context check on, in the same save; absent keeps them. Answers the flow as `GET` does. Refusals: `unknown_pack`, `unknown_flow`, `unknown_stage` (also for a `freshCheck` id), `fresh_check_not_allowed` (the stage has no document gate of role spec or design), `unknown_agent`, `agent_archived`, `bad_assignment`, `mixed_runtime` (the agents of one flow must share a runtime) |

A write publishes `{"type": "agents"}`.

### Skills library

Reusable skills agents list in `skills`. A skill is a folder `<data>/skills/<name>/` with a `SKILL.md` whose frontmatter has `name` (the folder's name) and `description`, and the files it uses. At most 1 MiB and 200 files per skill; no symbolic links and no hidden files. Names follow the job-name rule. Jobs do not use them yet (a copy into the worktree comes with the runtime skill target).

| Method and path | Does |
|---|---|
| `GET /skills` | `{skills}`: `name`, `description`, `files`, `bytes`, `sha256` (over paths and contents), `usedBy` (names of the agents that list it) |
| `GET /skills/{name}` | The same plus `fileList` (`path`, `bytes`) and `text` (the `SKILL.md`). `no_such_skill` (404) |
| `POST /skills` | `{name, text}` (the `SKILL.md`) or `{name, files: [{path, text \| base64}]}`, plus `replace: true` to overwrite a skill of that name. Refusals: `bad_name`, `bad_skill` (header, path, hidden file, no `SKILL.md`), `too_large` (413), `name_taken` |
| `DELETE /skills/{name}` | Refused with `skill_in_use` (409) while an agent lists it |

Creating or editing an agent that lists a skill the library does not have is refused with `unknown_skill` (422).

### Reviews, TODOs, time, backups

| Method and path | Does |
|---|---|
| `GET /reviews`, `POST /reviews/seen {items}` | Automatic approvals to look at, and marking them seen |
| `GET /todos`, `POST /todos`, `PUT /todos/{id}`, `DELETE /todos/{id}` | TODOs |
| `POST /todos/{id}/complete`, `POST /todos/{id}/reschedule {due}`, `POST /todos/{id}/check` | Complete, reschedule, run a condition check now |
| `GET /worklog?days=N[&group=agent]` | Job and user time per day and ticket, with delivery metrics; `group=agent` lists the days by saved agent (name) instead of ticket |
| `GET /backups`, `POST /backups` | List and create backups |

### Git access, plugins, toolchains, Open in

| Method and path | Does |
|---|---|
| `GET /git/hosts`, `PUT /git/hosts` | Hosts, their provider and credential source (`providers` lists the host plugins). `GET` also lists the implicit rows of the Git providers that are on, each with `from` = the plugin's name; they are never saved, and `PUT` takes only the user's rows (a row carrying `from` is refused) |
| `POST /git/doctor`, `POST /git/known-hosts`, `POST /git/login` | Check access to a host, accept a host key (re-scanned, same fingerprint required), start a browser login in a terminal |
| `POST /git/credential` | The git credential helper's endpoint (local user or a job token with `credential`) |
| `GET /plugins` | `{plugins: [{name, kind, version, builtin, source?}], restartNeeded, installEnabled}`; `source` is `{url, ref, path, sha, digest, installedAt, updatedAt}` for a plugin xherd installed (also on `/state` plugins) |
| `POST /plugins/preview` | Body `{url, ref?, path?}`. Fetches the repository into a scratch folder and answers what installing would do: `name`, `kind`, `version`, `sha`, `digest`, size, what it runs, warnings, `replaces`, and `choices` when the repository holds several plugins (then pass `path`). Nothing is installed |
| `POST /plugins/install` | Body `{url, ref?, path?, digest}`: fetches again and installs only when the digest equals the previewed one (409 `changed_since_preview` otherwise; no `digest` is 400 `usage`). The plugin loads after a restart |
| `POST /plugins/{name}/update` | Body `{ref?, digest?}`. Without `digest`: the preview with the installed record and a `manifestDiff`. With it: applies that digest, keeping the old folder until the new one is recorded |
| `DELETE /plugins/{name}?forget=true` | Remove an installed or hand-copied external plugin; `forget` drops its `pluginSettings` and secrets. 409 `plugin_in_use` for a live job's runtime or host, 409 `builtin_plugin`, 404 `unknown_plugin` |
| `POST /admin/restart` | 202 `{restarting: true}`: the daemon stops gracefully and re-executes itself (same PID). Local user only |
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
| `GET /ws/term/{id}?cols=&rows=` | A job's tmux session. Binary frames carry terminal bytes both ways; a text frame `{"type": "resize", "cols", "rows"}` resizes |
| `GET /ws/setup/{id}` | The setup terminals: `git-login`, `plugin-login` (plugin logins and installs) |

### Jobs and runtimes

| Method and path | Does |
|---|---|
| `POST /hooks` | A runtime hook or `xherd report` (scope `hooks`). Reports carry a sequence number; an older one is ignored |
| `POST /xap` | An XAP call (scope `xap`). See [XAP](xap.md) |

## Changes

**Jobs replace agents (#141, breaking).** What the API called an agent is a *job*.

- Every `/api/v1/agents/…` route that served a job is now `/api/v1/jobs/…`, with the same handlers and bodies; `POST /jobs` takes what `POST /agents` took. `/state` carries `jobs`, the change feed's message type is `job`, and ids in bodies are `jobId` (the old `agentId` is still read as an input alias, so hooks queued before the upgrade replay).
- For one release a request to an old `/api/v1/agents/{x}/…` path whose `{x}` is a job answers `410 {"code": "moved", "location": "/api/v1/jobs/…"}`; an unknown `{x}` answers `404 unknown_job`. Error codes `unknown_agent` and `ambiguous_agent` are now `unknown_job` and `ambiguous_job`.
- Session tokens are of kind `job` (`XHERD_JOB_TOKEN`); tokens of kind `agent` issued before are still accepted.
- Config keys `maxConcurrentAgents` and `agentTokenBudget` (config and repo profiles) are now `maxConcurrentJobs` and `jobTokenBudget`; the old keys are read once and never written.
- The Herd view plugin protocol v1 is unchanged: its field is still `agents`, and holds the jobs.
- The `/agents` paths now serve saved agents (see above): `GET` and `POST /agents` are the agent list and create, and `/agents/{x}` resolves a saved agent first. `GET /agents` used to answer `410` pointing at `/jobs`; it no longer does. Only a `{x}` that is not an agent but is a job still answers `410`, and a path an agent does not have answers `404 no_such_route`.
