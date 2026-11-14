# Plugins

xherd has two extension points. A **plugin** teaches xherd about a tool: an agent runtime, a git host, an issue tracker, a language toolchain. A **[flow pack](flow-packs.md)** defines the steps agents follow and the prompts that drive them. Both are data plus, where needed, commands in any language. Running your own spec-driven method under xherd, with or without a pack, is planned: see [Add xherd support to your SDD](integrate-your-sdd.md).

A plugin is a folder with an `xherd-plugin.json` manifest. xherd loads built-in plugins first, then every `<data root>/plugins/*/xherd-plugin.json`; an external plugin overrides a built-in with the same name. A manifest that fails to parse is listed as *broken* in Settings → Plugins with its error, and does not stop xherd.

| Built-in | Kind | Does |
|---|---|---|
| `claude-code` | runtime | Launch argv, permission modes, hook file and event map, trust file, transcript lookup, token usage, read-only check command |
| `opencode` | runtime | The same for opencode: permission modes through `OPENCODE_PERMISSION`, a JavaScript plugin file as its hook file, the first prompt on the launch line, transcripts through `opencode export` |
| `codex` | runtime | The same for the OpenAI Codex CLI: permission modes as Codex sandbox and approval flags, `.codex/hooks.json`, trust in `~/.codex/config.toml`, the first prompt on the launch line (also when resuming), transcripts under `~/.codex/sessions` |
| `github` | host | Git access for GitHub, and pull request lookup, status, checks, comments, replies and templates through the `gh` CLI |
| `gitlab` | host | The same for GitLab merge requests through `glab api` |
| `bitbucket` | host | Git access only (a token kept by xherd); no pull request support yet |
| `jira`, `github-issues`, `gitlab-issues` | tracker | Ticket keys as links, the Ticket tab (reading the ticket and its comments), posting a summary you accepted |
| `sdkman`, `nvm`, `uv`, `go`, `rustup` | toolchain | Which version files they read and how to find or install a tool |
| `herd-tiles` | view | A layout of the Herd page: one tile per agent, grouped by repository. The example of a [Herd view plugin](#herd-view-plugins) |

Kinds in a manifest: `runtime`, `host`, `toolchain`, `tracker`, `view`, `flow`, `notifier`, `panel`. Today xherd uses runtime, host, tracker, toolchain and view plugins; the others are accepted by the manifest but nothing consumes them yet.

Every plugin can be switched on or off in Settings → Plugins (or the Welcome screen), as long as one plugin of each kind stays on; host plugins (shown as **Git providers**), trackers and view plugins (shown as **Herd views**) are optional, so having none is fine (with no Git provider on, no agent gets pull request lookup, polling, comments or templates). Its card there opens its options in a dialog: settings, secrets, checks with their fixes, its login, and for a toolchain the versions.

## Manifest

```json
{
  "name": "my-host",
  "kind": "host",
  "version": "0.1.0",
  "label": "My Host",
  "description": "Pull requests on my forge.",
  "settings": [
    { "key": "binary", "label": "CLI binary", "default": "myforge" },
    { "key": "token", "label": "Token", "secret": true }
  ],
  "commands": { "pr-for-branch": ["./pr-for-branch.sh"] },
  "checks": [ { "name": "cli", "argv": ["myforge", "--version"], "fix": ["brew", "install", "myforge"] } ],
  "timeoutSeconds": 30
}
```

| Field | Meaning |
|---|---|
| `name` | Lowercase letters, digits and `-`, at most 64 characters |
| `min_xherd_version` | Refuse to load on older xherd |
| `settings` | Shown in the plugin's options. `secret: true` values are write-only in the UI (it shows whether one is set, never the value), live in `secrets.json`, apply at once, and are passed to commands separately from plain settings. Plain settings apply at the next daemon start |
| `commands` | Command name → argv |
| `hooks` | Event name → argv; the output is ignored |
| `checks` | Each runs an argv; pass = exit 0 within 10 s. They appear in Setup checks. `fix` is a command offered to the user; `optional` checks do not block |
| `requires` | Command-line tools the plugin needs, with a check, a container install and native instructions ([below](#tools-a-plugin-needs-requires)) |
| `login` | Argv of an interactive login, run in a terminal the UI shows (*Log in* on the plugin's card and in its options). Whether it is logged in is the check named `login`; an enabled plugin whose `login` check fails holds the Welcome Plugins step. The built-in `github` has one (`gh auth login --hostname github.com --git-protocol https --web`, check `gh auth status`) |
| `build` | Install-time build argv |
| `timeoutSeconds` | Per-command timeout (default 60) |
| `data`, `instructions` | Kind-specific data and prompt fragments |

## Command protocol

xherd runs a command's argv in the agent's worktree (as the agents' user in the container), writes one JSON object to its stdin — `{command, input, settings, secrets, agent}` — and reads one JSON value from stdout. stderr goes to the log with secrets redacted. A non-zero exit or output of the wrong shape is a failure of that call; xherd logs it and carries on (a failed PR poll never blocks an agent).

A host plugin answers these commands; the built-in `github` plugin implements them in Rust through `gh`, the built-in `gitlab` plugin through `glab api` (GitLab's REST API; merge requests, their approvals, the head pipeline's jobs and discussions), and an external plugin answers them through the protocol. Which plugin serves an agent is decided by the host of its remote: the provider on its row in Settings → Git access, else the plugin whose `data.git` `hosts` claim the name, else `github`. The built-in `bitbucket` has git access only and answers these commands with an error saying so:

| Command | Input | Answer |
|---|---|---|
| `pr-for-branch` | `{branch, worktree}` | `{url}` or `null` |
| `pr-status`, `pr-checks` | the PR | `pr-status`: `{state, mergeable?, base?, reviewers?: [{name, state}]}` (review state `pending`, `approved`, `changes-requested`, `commented` or `dismissed`); `pr-checks`: `[{name, bucket, workflow?, link?}]`. The parsers are `parse_status` and `parse_checks` in `crates/xherd-plugin/src/host.rs`. When `mergeable` is missing or unknown, xherd fetches `origin` in the worktree and tries the merge into `origin/<base>` itself (`git merge-tree`, nothing in the worktree changes) |
| `pr-comments` | the PR | Comments with ids of the form `<kind>:<id>` |
| `pr-reply` | a comment and text | Posts the reply. xherd appends its marker so its own replies are recognised |
| `pr-template` | the worktree | The repository's pull request templates |

Treat the shapes in `host.rs` as the contract; this table is a summary.

A host plugin also declares its git access in `data.git` (the built-in `github`, `gitlab` and `bitbucket` do; the reading code is `crates/xherd-plugin/src/git.rs`). A host in Settings → Git access is bound to a provider (the `provider` field, a plugin name; empty = the first provider claiming its name) and, with the credential `provider`, takes these from it:

- `hosts`: host names it claims (`github.com`, `*gitlab*`; `*` matches any run of characters). They decide the token user and which provider's API checks the doctor uses for a host.
- `credential`: `helper` (the git credential helper, e.g. `!gh auth git-credential`; absent for a provider with no CLI, whose token xherd keeps) and `tokenUser` (the name git sends with a token; a host no provider claims gets `x-access-token`).
- `login`: the browser login argv; `{host}` is replaced. Without it the host takes a pasted token.
- `api`: the doctor's API checks: `base` (`{host}` is replaced; https only), `user` (`path`, and optionally `scopesHeader` + `requireScope`), and optionally `repo` (`path` with `{path}`, `ssoHeader`, and the `onSso` / `onNotFound` verdicts: a `detail` and a `fix`, which may use `{url}`).

**Implicit rows.** A host plugin that is on and has a credential `helper` gives Git access a row for each name in its `hosts` without a `*` (the built-in `github`: `github.com`), credential `provider`, HTTPS and SSH. The doctor uses it like a saved row, and so does the managed gitconfig in the container; natively it is left out of the managed gitconfig (which the user's own `~/.gitconfig` includes, so it would override the user's own credential helper for that host), and a saved row is needed for xherd to set the helper there. It is never written to `config.json`; `GET /git/hosts` lists it with `from` set to the plugin's name. A row the user saved for the same host replaces it, and switching the plugin off removes it. A pattern such as `*gitlab*` gives none.

Values end up in a git config file and on command lines, so a control character anywhere is refused when the manifest is read. Transport (SSH, known hosts, token storage, the managed gitconfig) stays in xherd. A plugin's options page (Settings → Plugins) lists the hosts that use it.

## Issue trackers (`data.tracker`)

A `tracker` plugin says how text in the UI becomes a link to the ticket, how to read a ticket, and how to post a comment you wrote. Agents never call a tracker, and xherd posts nothing by itself.

```json
"settings": [{ "key": "baseUrl", "label": "Jira site", "required": true }],
"data": { "tracker": { "links": [
  { "pattern": "\\b[A-Z][A-Z0-9]{1,9}-\\d+\\b", "url": "{baseUrl}/browse/{0}" }
] } }
```

`pattern` is a regular expression run in the browser over agent-written text and the ticket key on the agent page (outside code and existing links). `url` is `https://…` or starts with `{baseUrl}` (the plugin's `baseUrl` setting, https only; a rule whose base URL is missing is left out); `{0}` is the match and `{1}`…`{9}` its groups, percent-encoded. A manifest is refused when a pattern does not compile with the linear-time `regex` crate (no backreferences or lookaround), is over 200 characters, or repeats a repeated group (`(a+)+`), and when a URL uses a group the pattern lacks. The built-in `jira` (`PAY-412`), `github-issues` (`owner/repo#12`) and `gitlab-issues` (`group/project#12`, `baseUrl` default `https://gitlab.com`) use it. Having no tracker is allowed.

A tracker can also read the agent's ticket, shown in the agent page's **Ticket** tab (title, state, description and comments, oldest first; reading never posts anything). Two more fields in `data.tracker` say which tickets it reads: `key`, a regular expression with exactly one group whose match is the issue reference (`^(?:GH-)?(\\d+)$` turns `GH-12` or `12` into `12`; the reference must be a plain name, since it ends up in a command line), `url`, a regular expression over a whole issue URL with exactly one group, the issue reference (it may start with `{baseUrl}`, the plugin's setting, matched literally; without that setting such a pattern reads nothing), and `hosts`, the git host names it serves (`github.com`, `*gitlab*`). An agent's ticket that is a URL goes to the first enabled tracker whose `url` reads it, and that reference gives the agent's key (`#12` for a number, else the reference upper-cased), which starts its branch name. A key goes to the enabled tracker that serves the host of the worktree's `origin` and reads the key, then to one that serves any host. Built in: `jira` reads `{baseUrl}/browse/<KEY>`, `github-issues` `https://github.com/<owner>/<repo>/issues/<n>` (keys `12`, `#12`, `GH-12`), `gitlab-issues` `https://<host>/<group…>/<project>/-/issues/<n>` (keys `12`, `#12`, `GL-12`). For a URL, `github-issues` adds `-R <host>/<owner>/<repo>` and `gitlab-issues` uses the URL's host and project instead of the worktree's `origin`; the token kept for the remote's host is only used when the URL is on that host. The built-in `github-issues` runs `gh issue view <n> --json title,body,state,url,author,comments`; `gitlab-issues` runs `glab api` on `projects/<path>/issues/<n>` and its notes (system notes left out). Both use the plugin's `gh` / `glab` and `token` settings or, without a token, the one kept for the remote's host in Settings → Git access. `jira` reads through Atlassian's CLI: `acli jira workitem view <KEY> --json --fields summary,status,description,reporter` and `acli jira workitem comment list --key <KEY> --json --paginate`, using `acli`'s own login (`acli jira auth login --web`, status `acli jira auth status`; the plugin has the usual Log in button and an optional `login` check) and the plugin's `acli` setting. Its description and comments are Atlassian Document Format, which xherd turns into Markdown (headings, lists, links, code, quotes, tables, mentions; an unknown node keeps its text). The ticket's link is `<baseUrl>/browse/<KEY>` from the plugin's `baseUrl` setting (the `self` URL in the output is an internal Atlassian host). The container install of `acli` downloads Atlassian's binary into `~/.local/bin`; natively the instructions are shown (Homebrew on macOS).

**Posting a summary.** When a pull request is open, the agent page offers to post a summary on the ticket (only if an enabled tracker reads it and nothing was posted; "Not now" is remembered in the browser), and the packs do not touch tickets at all: the built-in ship prompt only writes `pr.md`, which becomes the PR body. The Ticket tab can post a short summary of what the agent did as a comment (R-308: only the user's accept posts anything). *Draft a summary* asks xherd for a draft built from the agent's finished tasks, commits and pull request (`xherd_core::ticket::summary`, no model, no ticket text in it); the user reads and edits it, and *Post to the ticket* sends exactly that text (`POST /agents/{id}/ticket-comment`, at most 10 000 characters, no control characters). The built-in trackers run `gh issue comment <n> --body <text>`, `glab api -X POST projects/<path>/issues/<n>/notes -f body=<text>` (both add xherd's hidden marker `<!-- xherd -->`) and `acli jira workitem comment create --key <KEY> --body <text>`; the text is one argument, never through a shell. An external tracker answers `ticket-comment` (stdin `{command, input: {key, issue, url, body, worktree}}`, `url` null for a ticket given as a key; no output needed). The post is logged as the event `ticket-comment` with its length, not its text. Jira takes plain text here: how it handles newlines in `--body` is unchecked.

An external tracker answers the command `ticket-get` (stdin `{command, input: {key, issue, url, worktree}}`, stdout `{title, body?, state?, url?, author?, comments?: [{id, kind, author, body, url?, createdAt?}]}`). Ticket and comment text is written by other people: the UI renders it through the sanitiser, and anything that hands it to an agent must wrap it as untrusted (R-1201).

## Herd view plugins

A `view` plugin adds a layout to the Herd page: a web page of your own, drawn with any tools you like, that receives the agents and can open one. Each enabled view plugin is a choice in the Herd's **Layout** switch after the built-in ones (*List*, *By stage*), labelled with the manifest's `label` (else its name), chosen in the URL as `?layout=plugin:<name>`. Switch it off in Settings → Plugins (**Herd views**) to remove it; having none is fine.

```json
{
  "name": "my-view",
  "kind": "view",
  "version": "0.1.0",
  "label": "My view",
  "description": "Agents as a timeline.",
  "data": { "view": { "entry": "index.html" } }
}
```

A view plugin is **files only**: a manifest with `commands`, `hooks`, `checks`, `requires`, `login`, `build` or `instructions` is refused. `data.view.entry` is the page to show, a relative `.html` path inside the folder. Nothing of a view runs in the daemon.

**Files.** The daemon serves the plugin's folder at `/api/v1/plugins/<name>/view/<path>` (only while the plugin is on; no login needed, so put nothing private in it: every file there is served, the manifest included). Paths are plain: no `..`, no hidden files or folders (a segment starting with `.`), nothing that resolves outside the folder through a symlink, at most 16 MiB per file. The type comes from the extension (`html`, `js`/`mjs`, `css`, `json`, `map`, `svg`, `png`, `jpg`/`jpeg`, `gif`, `webp`, `ico`, `woff`/`woff2`, `ttf`, `txt`; anything else is `application/octet-stream`).

**The sandbox.** The Herd page shows the entry in an `<iframe sandbox="allow-scripts">`, and every view file is also served with a `Content-Security-Policy` that starts with `sandbox allow-scripts`, so the page has an opaque origin even when opened in a tab of its own. It cannot read xherd's API, cookies or storage, and the policy allows:

| Allowed | Not allowed |
|---|---|
| Scripts, styles, images and fonts from the plugin's own files and the view kit (by URL path, so not from the rest of `/api/v1/`) | Inline `<script>` and `on…=` attributes, `eval`, scripts from anywhere else |
| Inline `style` attributes and `<style>`, `data:` images | `fetch`, XHR, WebSocket, EventSource (`connect-src 'none'`) |
| Messages to and from the Herd page (below) | Forms, nested frames, workers, `<base>`, `localStorage` and cookies (opaque origin), opening windows or navigating the Herd page |

So a view gets its data only by message, and does what it shows with the DOM. Agent fields (names, branches, tickets) are written by people and agents: draw them as text (`textContent`), never as HTML.

**Protocol v1.** Every message is a plain object with `xherdView: 1` and a `kind`. The view sends `{kind: "ready"}` once it listens; from then on the Herd page sends `{kind: "data", agents, flows, theme}` after each change (agents, flows or the visible theme). The view may send `{kind: "open", agentId}` to open that agent's page; anything else, and an `open` of an unknown agent, is ignored. The page accepts messages only from the frame's window with the origin `null`, and posts to that window only.

`theme` is `light` or `dark`. `flows` are the packs' flows (`{pack, flow, stages: [{id, role, class, gate}]}`, as in `GET /state`). `agents` holds every agent (finished ones too, unfiltered) as a `ViewAgent`, and nothing else of it: no prompts, messages, question texts, documents, comments or paths.

| Field | Meaning |
|---|---|
| `id`, `name` | The agent's id (for `open`) and name |
| `status`, `statusSince` | `queued`, `running`, `waiting-input`, `blocked`, `idle`, `parked`, `paused`, `watching`, `done`, `cancelled`, `error`; since when (ms) |
| `createdAt`, `lastActivity` | Times in ms since the epoch |
| `repo`, `branch` | The repository's folder name, the branch |
| `ticket` | `{key?, url?}` when it has one |
| `runtime`, `flow`, `stage` | When known |
| `stageName`, `waitsOn` | The current stage's label, and what its open gate waits on (document names, a commit id, a count) |
| `gate` | The open gate's kind only (`docs`, `commit`, `question`) |
| `needsYou` | Whether it waits for the user |
| `hue` | The agent's colour as a CSS `hsl(…)`, the one the rest of the UI uses |
| `prUrl`, `prState`, `prCi` | The pull request, its state and checks, when there is one |

A field with no value is left out. `gate` may also be `pr-triage` or `pr-replies` (pull request comment rounds).

**The view kit.** Two files at `/api/v1/view-kit/` save writing the protocol and the styles yourself:

- `xherd-view.js`, a classic script that defines the global `xherdView`: `xherdView.onData(cb)` calls `cb({agents, flows, theme})` for every `data` message (a callback added after the first message gets the last one at once), `xherdView.open(agentId)` asks to open an agent. It says `ready` when it loads and sets `data-theme` on `<html>` before each callback. Load it before your own scripts.
- `xherd.css`, xherd's design tokens and component classes (`app.css`), which follow `data-theme`, so the view matches the UI in both themes.

Reference them by absolute path (`/api/v1/view-kit/xherd-view.js`); your own files by relative path.

**Example.** The built-in `herd-tiles` ([`plugins/herd-tiles`](../../plugins/herd-tiles)) is a whole view in plain HTML, CSS and JavaScript with no build step, about 150 lines: copy its folder to start.

```html
<link rel="stylesheet" href="/api/v1/view-kit/xherd.css">
<link rel="stylesheet" href="tiles.css">
…
<script src="/api/v1/view-kit/xherd-view.js"></script>
<script src="tiles.js"></script>
```

```js
xherdView.onData(function (d) {
  root.replaceChildren();
  d.agents.forEach(function (a) {
    var b = document.createElement('button');
    b.textContent = a.name;
    b.onclick = function () { xherdView.open(a.id); };
    root.appendChild(b);
  });
});
```

**With Vite and Svelte** (or any bundler): build a static page, and make it fit the policy. In `vite.config.js` set `base: './'` (the files are served under the plugin's path) and `build: { modulePreload: false }` (Vite's preload helper uses `fetch`, which is blocked). In `index.html` load the kit by absolute URL, as a plain script, before your module:

```html
<link rel="stylesheet" href="/api/v1/view-kit/xherd.css">
<script src="/api/v1/view-kit/xherd-view.js"></script>
<div id="app"></div>
<script type="module" src="/src/main.js"></script>
```

```svelte
<script>
  let agents = $state([]);
  window.xherdView.onData((d) => (agents = d.agents));
</script>
{#each agents as a (a.id)}
  <button onclick={() => window.xherdView.open(a.id)}>{a.name}</button>
{/each}
```

`vite build` then gives `dist/` with `index.html` and `assets/` (Vite warns that the kit's URLs do not exist at build time; that is expected). The plugin folder is the contents of `dist/` plus `xherd-plugin.json` with `"entry": "index.html"`. `vite dev` does not work as a view (its client uses inline scripts and a WebSocket): build, copy the folder to `<data root>/plugins/<name>/` and restart the daemon. After a later build, reloading the Herd page is enough natively (files are served with `no-cache`); in the container the daemon serves a copy made at start, so restart it again.

## Tools a plugin needs (`requires`)

Any plugin can list the command-line tools it needs:

```json
"requires": [{
  "name": "glab",
  "check": ["glab", "--version"],
  "optional": true,
  "install": ["sh", "-c", "…"],
  "instructions": { "macos": "brew install glab", "linux": "See https://…", "any": "See https://…" }
}]
```

Each entry is a check named `requires:<name>` (`check` defaults to `command -v <name>`; `optional` means a missing tool is reported but never blocks). When it fails, the container offers the `install` argv from the UI, run in a terminal into the agents' home (the same session as plugin logins); a native run shows the `instructions` for its OS (`macos`, `linux`, else `any`) and never installs anything on the host (R-653). `install` must name a program on `PATH` (no `./` script), `name` is a plain word, and a control character anywhere is refused. The built-in `github` (`gh`) and `gitlab` (`glab`) use it, as do `jira` (`acli`), the version managers and the three runtimes (`claude`, `opencode`, `codex`). The container image already carries all of these; a runtime's container install is `npm install -g --prefix "$HOME/.local" <package>@<version>` (the agents' `PATH` starts with `~/.local/bin`, and `/usr/local` is not theirs to write), pinned to the version the image is built with and xherd was checked against (a test keeps it equal to the Dockerfile's `ARG`). Natively xherd shows the instructions and installs nothing. The review of an external plugin's install argv before it runs belongs to the install-from-GitHub flow (R-542), which is not built.

## Toolchain plugins

A toolchain plugin's `data.toolchain` lists tools. For each: which version files to read (with a format from a fixed set such as `sdkmanrc`), a `resolve` argv that prints the directory to put on `PATH` when the version is installed, an `install` argv, an optional `list` argv (one installable version per line on stdout, best first; fills the version dropdown on the plugin's page in Settings → Plugins, container only), and the environment to set. `{v}` is the version, `{bin}` and `{home}` the directories. Commands are shell snippets that take the version as `"$1"`. The first plugin that claims a tool wins. Installing happens only in the container. An optional `installed` argv lists the versions already there, one per line (local commands only, so it runs natively too): the plugin's options page and the Welcome card show them. The version managers also declare themselves with `requires` (above), so a missing manager shows install instructions.

## Runtime plugins

A runtime plugin's `data.terminal` says how to run an agent CLI in tmux:

- `launch`: the argv.
- `permissionModes`: for each of `ask`, `edits`, `auto`, `all`, the explicit flags: a list of arguments, or `{"args": [...], "env": {...}}` for a runtime that reads its permissions from the environment (xherd starts it through `env K=V`). `ask` and `edits` are required and nothing else is allowed, so a mode can never be a free-form string.
- `hooks`: the file the runtime reads hooks from and a map from its events to xherd's (`session-start`, `user-prompt-submit`, `post-tool-use`, `notification`, `stop`). `format` says how it is written: `claude-settings` (default: a JSON settings file whose `hooks` map runs `xherd hook`; the user's entries are kept), `codex-hooks` (the same `hooks` map without a `permissions` block, e.g. `.codex/hooks.json`) or `opencode-plugin` (a JavaScript plugin owned by xherd, e.g. `.opencode/plugin/xherd.js`; its events are `start`, `chat.message`, `tool.execute.after`, `permission.asked`, `question.asked`, `session.idle`; it also allows the XAP calls and the command folder in opencode's permissions). Either way the file is kept out of git.
- `promptArgs`: launch arguments that give a new session its first prompt (`{prompt}` once), for a runtime whose input is not ready when its start hook fires. Without them xherd types the prompt at `session-start`.
- `trust`: where the runtime remembers trusted folders, so xherd can trust each new worktree: `file`, the `projects` map and the `key` set in a folder's entry, to `value` (default `true`); `format` is `json` (default) or `toml` (edited in place, comments kept: Codex's `[projects."<path>"] trust_level = "trusted"`).
- `resume` and `usage`: how to continue the last conversation (`args` with `{session}`), where its transcript is, and how to count tokens in it (for budgets). The transcript is a file (`transcripts` folder + `file` name, which may hold one `*`, `depth` folders down: 1 by default, Codex's `<year>/<month>/<day>` is 3) or what an `export` command writes, `{session}` once: on stdout, or into `{file}` (a path xherd picks in the worktree's `.xherd/`) when a pipe is not reliable. `promptEnv` (variables with `{prompt}` and `{session}`) gives a continued session its first prompt when the runtime ignores `promptArgs` then; `resume.promptArgs` (`{prompt}` once, after `args`) does it on the launch line instead (`codex resume <id> <prompt>`). `usage.fields` names a counter in each record, or a JSON pointer inside it (`/cache/write`); a transcript that is one JSON document is read as a single record.
- `modelArgs`: launch arguments that choose the model, `{model}` once (`["--model", "{model}"]`); they go after the permission mode's flags, so resumed and fresh sessions keep the model. Without them agents of the runtime cannot choose a model. `models` gives the UI its suggestions, in one of two forms: a fixed list, `[{"id": "sonnet", "label": "Sonnet (latest)", "tier": "balanced"}]` (`label` and `tier` optional, `tier` shown as is), or a command, `{"command": "opencode models"}`, run with `sh -c` as the agents' user (so pipes, `grep`, `jq`, `awk` work; 30 s limit) that prints one model per line as `id`, `id<TAB>label` or `id<TAB>label<TAB>tier` (odd ids are skipped, at most 500). Codex's is `codex debug models | jq -r '…'`. A command that fails gives no suggestions and its reason; either way they are only suggestions, any plain model name can be typed. `models` needs `modelArgs`.
- `check`: how to run the runtime once, headless and read-only, to answer a question (TODO conditions). `argv` holds `{prompt}` and `{system}` once each; `modelArgs` (with `{model}`) are added when a model is chosen. List only tools that cannot change anything: xherd relies on it. The run must end with a line `XHERD_RESULT {"met": true, "summary": "…"}`. A runtime without `check` cannot check conditions.

Claude Code, opencode and Codex are built in. An agent's runtime is chosen when it is created (`xherd new --runtime`, the New agent form, a repo profile's `runtime`, else the `defaultRuntime` setting) and stays the same for its whole life. The pack must have commands for it (`install.<runtime>` in `xherd-flow.yaml`).

## External plugins

Put the plugin's folder in `<data root>/plugins/<name>/` and restart the daemon. In the container the daemon copies that folder at start to one the agents' user can read, and runs the plugin's commands, checks and hooks as that user (see [Security](../user/security.md#plugins)); its own config and state folders are `~/.local/share/xherd-plugins/{config,state}/<name>` in the agents' home. The name `git` is reserved. Installing from the UI or from GitHub is not built yet.
