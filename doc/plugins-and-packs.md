# Plugins and flow packs

xherd has two extension points. A **plugin** teaches xherd about a tool: an agent runtime, a git host, a language toolchain. A **flow pack** defines the steps agents follow and the prompts that drive them. Both are data plus, where needed, commands in any language. If you only want your own spec-driven method to run under xherd with gates and approvals, you do not need a pack: see [Add xherd support to your SDD](integrate-your-sdd.md).

## Plugins

A plugin is a folder with an `xherd-plugin.json` manifest. xherd loads built-in plugins first, then every `<data root>/plugins/*/xherd-plugin.json`; an external plugin overrides a built-in with the same name. A manifest that fails to parse is listed as *broken* in Settings → Plugins with its error, and does not stop xherd.

| Built-in | Kind | Does |
|---|---|---|
| `claude-code` | runtime | Launch argv, permission modes, hook file and event map, trust file, transcript lookup, token usage, read-only check command |
| `opencode` | runtime | The same for opencode: permission modes through `OPENCODE_PERMISSION`, a JavaScript plugin file as its hook file, the first prompt on the launch line, transcripts through `opencode export` |
| `codex` | runtime | The same for the OpenAI Codex CLI: permission modes as Codex sandbox and approval flags, `.codex/hooks.json`, trust in `~/.codex/config.toml`, the first prompt on the launch line (also when resuming), transcripts under `~/.codex/sessions` |
| `github` | host | Pull request lookup, status, checks, comments, replies and templates through the `gh` CLI |
| `sdkman`, `nvm`, `uv`, `go`, `rustup` | toolchain | Which version files they read and how to find or install a tool |

Kinds in a manifest: `runtime`, `host`, `toolchain`, `tracker`, `flow`, `notifier`, `panel`. Today xherd uses runtime, host and toolchain plugins; the others are accepted by the manifest but nothing consumes them yet.

### Manifest

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
| `settings` | Shown in Settings. `secret: true` values live in `secrets.json`, are not editable in the UI yet, and are passed to commands separately from plain settings |
| `commands` | Command name → argv |
| `hooks` | Event name → argv; the output is ignored |
| `checks` | Each runs an argv; pass = exit 0 within 10 s. They appear in Setup checks. `fix` is a command offered to the user; `optional` checks do not block |
| `login` | Argv of an interactive login, run in a terminal the UI shows. Whether it is logged in is the check named `login` |
| `build` | Install-time build argv |
| `timeoutSeconds` | Per-command timeout (default 60) |
| `data`, `instructions` | Kind-specific data and prompt fragments |

### Command protocol

xherd runs a command's argv in the agent's worktree (as the agents' user in the container), writes one JSON object to its stdin — `{command, input, settings, secrets, agent}` — and reads one JSON value from stdout. stderr goes to the log with secrets redacted. A non-zero exit or output of the wrong shape is a failure of that call; xherd logs it and carries on (a failed PR poll never blocks an agent).

A host plugin answers these commands; the built-in `github` plugin implements them in Rust and an external plugin answers them through the protocol:

| Command | Input | Answer |
|---|---|---|
| `pr-for-branch` | `{branch, worktree}` | `{url}` or `null` |
| `pr-status`, `pr-checks` | the PR | `pr-status`: `{state, mergeable?, base?, reviewers?: [{name, state}]}` (review state `pending`, `approved`, `changes-requested`, `commented` or `dismissed`); `pr-checks`: `[{name, bucket, workflow?, link?}]`. The parsers are `parse_status` and `parse_checks` in `crates/xherd-plugin/src/host.rs`. When `mergeable` is missing or unknown, xherd fetches `origin` in the worktree and tries the merge into `origin/<base>` itself (`git merge-tree`, nothing in the worktree changes) |
| `pr-comments` | the PR | Comments with ids of the form `<kind>:<id>` |
| `pr-reply` | a comment and text | Posts the reply. xherd appends its marker so its own replies are recognised |
| `pr-template` | the worktree | The repository's pull request templates |

Treat the shapes in `host.rs` as the contract; this table is a summary.

### Toolchain plugins

A toolchain plugin's `data.toolchain` lists tools. For each: which version files to read (with a format from a fixed set such as `sdkmanrc`), a `resolve` argv that prints the directory to put on `PATH` when the version is installed, an `install` argv, an optional `list` argv (one installable version per line on stdout, best first; fills the version dropdown on the plugin's page in Settings → Plugins, container only), and the environment to set. `{v}` is the version, `{bin}` and `{home}` the directories. Commands are shell snippets that take the version as `"$1"`. The first plugin that claims a tool wins. Installing happens only in the container.

### Runtime plugins

A runtime plugin's `data.terminal` says how to run an agent CLI in tmux:

- `launch`: the argv.
- `permissionModes`: for each of `ask`, `edits`, `auto`, `all`, the explicit flags: a list of arguments, or `{"args": [...], "env": {...}}` for a runtime that reads its permissions from the environment (xherd starts it through `env K=V`). `ask` and `edits` are required and nothing else is allowed, so a mode can never be a free-form string.
- `hooks`: the file the runtime reads hooks from and a map from its events to xherd's (`session-start`, `user-prompt-submit`, `post-tool-use`, `notification`, `stop`). `format` says how it is written: `claude-settings` (default: a JSON settings file whose `hooks` map runs `xherd hook`; the user's entries are kept), `codex-hooks` (the same `hooks` map without a `permissions` block, e.g. `.codex/hooks.json`) or `opencode-plugin` (a JavaScript plugin owned by xherd, e.g. `.opencode/plugin/xherd.js`; its events are `start`, `chat.message`, `tool.execute.after`, `permission.asked`, `question.asked`, `session.idle`; it also allows the XAP calls and the command folder in opencode's permissions). Either way the file is kept out of git.
- `promptArgs`: launch arguments that give a new session its first prompt (`{prompt}` once), for a runtime whose input is not ready when its start hook fires. Without them xherd types the prompt at `session-start`.
- `trust`: where the runtime remembers trusted folders, so xherd can trust each new worktree: `file`, the `projects` map and the `key` set in a folder's entry, to `value` (default `true`); `format` is `json` (default) or `toml` (edited in place, comments kept: Codex's `[projects."<path>"] trust_level = "trusted"`).
- `resume` and `usage`: how to continue the last conversation (`args` with `{session}`), where its transcript is, and how to count tokens in it (for budgets). The transcript is a file (`transcripts` folder + `file` name, which may hold one `*`, `depth` folders down: 1 by default, Codex's `<year>/<month>/<day>` is 3) or what an `export` command writes, `{session}` once: on stdout, or into `{file}` (a path xherd picks in the worktree's `.xherd/`) when a pipe is not reliable. `promptEnv` (variables with `{prompt}` and `{session}`) gives a continued session its first prompt when the runtime ignores `promptArgs` then; `resume.promptArgs` (`{prompt}` once, after `args`) does it on the launch line instead (`codex resume <id> <prompt>`). `usage.fields` names a counter in each record, or a JSON pointer inside it (`/cache/write`); a transcript that is one JSON document is read as a single record.
- `check`: how to run the runtime once, headless and read-only, to answer a question (TODO conditions). `argv` holds `{prompt}` and `{system}` once each; `modelArgs` (with `{model}`) are added when a model is chosen. List only tools that cannot change anything: xherd relies on it. The run must end with a line `XHERD_RESULT {"met": true, "summary": "…"}`. A runtime without `check` cannot check conditions.

Claude Code, opencode and Codex are built in. An agent's runtime is chosen when it is created (`xherd new --runtime`, the New agent form, a repo profile's `runtime`, else the `defaultRuntime` setting) and stays the same for its whole life. The pack must have commands for it (`install.<runtime>` in `xherd-flow.yaml`).

## Flow packs

A pack is a folder with `xherd-flow.yaml`, command prompts and a format description. The built-in `xherd` pack is in `packs/xherd/` and is embedded in the binary.

```yaml
name: xherd
version: 0.1.0
protocol: 1
install:
  claude-code:
    prefix: "/xherd:"
    commands: { from: commands/*.md, to: "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/xherd/{name}.md", format: command }
    args: "$ARGUMENTS"
commands: { resume: "{prefix}resume", handoff: "{prefix}handoff", status: "{prefix}status", prReview: "{prefix}pr-review" }
flows:
  feature:
    start: "{prefix}feature {key} {prompt}"
    stages:
      - { id: design, role: design, class: routine, gate: { docs: [design.md] }, next: [{ run: "{prefix}tasks" }] }
      # …
```

### Stages

Each stage has an `id`, a `role` (`explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`), a `class`, an optional `gate`, and `next` rules:

- **`gate`**: `{docs: [file, …]}` for a document gate, or `{kind: commit}` for commit proposals. A stage without a gate (the built-in `pr` stage) declares none: the agent is not stopped for approval there.
- **`class`**: `routine`, `critical`, `code` or `human-only`. Autonomy decides from it: `ask` approves nothing; `important` approves `routine` and `code` gates when no tripwire holds them; `critical` and `human-only` always wait. A stage with role `pr` is human-only whatever the manifest says.
- **`next`**: the command typed after approval, from a fixed vocabulary of conditions. `{prefix}`, `{key}`, `{prompt}`, `{commit}` and similar placeholders are filled in; `${…}` is left for the shell.
- **`loopUntil` / `then`**: repeat a stage until a condition (for example all tasks done), then move on.

### Command prompts

Each file in `commands/` becomes a runtime command (`/xherd:design`, …). Conventions that matter:

- Teach XAP as plain `xherd <call>` on one line in backticks, with concrete example values, never through a variable: Claude Code cannot check a command that uses a variable and asks every time. The worktree's `permissions.allow` lists the XAP subcommands.
- Every step starts a new session with no history. Each command reads `.xherd/spec/memory.md` first and rewrites it before its gate; the daemon refuses a gate or commit proposal without a valid one.
- Wrap nothing the daemon owns: it writes the `status:`/`evidence:`/`sha:` progress lines of `tasks.md` and `commits.md`, and makes approved commits itself.

`packs/xherd/FORMAT.md` documents the document formats (`tasks.md`, `commits.md`, `review.md`, `pr-review.md`, `memory.md`) that the daemon's parsers read.

### Installing

`xherd install` renders the pack's commands for each runtime (`{{prefix}}`, `{{args}}`, `{{dir}}`, `{{path:NAME}}` = where command NAME is installed for that runtime, `{{xap}}`, `{{format}}`) and writes them where the runtime reads commands. Format `command` writes each file as it is; `skill` (Codex) writes `<folder>/SKILL.md` with only `name` (the folder) and `description` in its frontmatter, plus `agents/openai.yaml` that keeps the skill from being used unless it is called by name. Hashes of what was written are kept, so a file you edited is left alone and the new version is written beside it with a `.xherd-new` suffix. In the container this runs at every daemon start.

### Limits today

Only the built-in `xherd` pack and its `feature` flow ship. Fields not implemented yet (`views`, `extends`, `parse`, …) are rejected by the manifest parser instead of being silently ignored. A new file in `packs/xherd/` must also be added to `xherd_flow::install::BUILTIN_FILES`.
