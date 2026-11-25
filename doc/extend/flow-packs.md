# Flow packs

A **flow pack** defines the stages jobs go through, where they stop for approval, and the prompts that drive each step. A pack is a folder with `xherd-flow.yaml`, command prompts and a description of the document formats. The built-in `xherd` pack is in `packs/xherd/` and is embedded in the binary. Another pack is loaded from a **flow plugin**, together with the saved agents and stage assignments that go with it (see [Workflow plugins](plugins.md#workflow-plugins-kind-flow) and [Plugin packs](#plugin-packs)); for running your own spec-driven method under xherd, see also [Add xherd support to your SDD](integrate-your-sdd.md); to teach xherd about a tool, see [Plugins](plugins.md).

```yaml
name: xherd
version: 0.1.0
protocol: 1
install:
  claude-code:
    prefix: "/xherd:"
    commands: { from: commands/*.md, to: "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/xherd/{name}.md", format: command }
    args: "$ARGUMENTS"
commands: { resume: "{prefix}resume", handoff: "{prefix}handoff", status: "{prefix}status", prReview: "{prefix}pr-review", conflicts: "{prefix}conflicts {base}" }
flows:
  feature:
    start: "{prefix}feature {key} {prompt}"
    stages:
      - { id: design, role: design, class: routine, gate: { docs: [design.md] }, next: [{ run: "{prefix}tasks" }] }
      # …
```

Top-level fields:

| Field | Meaning |
|---|---|
| `name`, `version`, `protocol` | The pack, its version, and the XAP major version it speaks |
| `specDir` | Where the documents live (`private`: `.xherd/spec/` in the worktree, out of git) |
| `private` | Files of the spec folder that are xherd's working files, not documents |
| `memoryNote` | `required` or `off` (default `off`). With `required`, `gate open` and `commit propose` are refused without a valid `.xherd/spec/memory.md` (four `##` sections, at most 80 lines); the built-in pack is `required`. With `off` (the default) xherd checks no memory note |
| `install.<runtime>` | Per runtime: `prefix` (how a command is called: `/xherd:` for Claude Code, `/xherd/` for opencode, `$xherd-` for Codex, `/xherd-` for Pi), `commands` (`from` glob, `to` destination with `{name}`, `format` `command` or `skill`), `args` (how the runtime passes arguments), `slashEnters` (Enters needed to run a command from the completion list). A job can use a runtime only if the pack has an entry for it |
| `commands` | Named commands xherd sends itself: `resume`, `handoff`, `status`, `prReview`, `conflicts` (sent by *Resolve conflicts*; `{base}` is the PR's base branch as `origin/<base>`; without it the action is refused), `ask` (sent by *Ask* with the user's text as `{prompt}`; without it the bare text is sent) |
| `flows` | Flows by name; each has `start` (the first command, with `{key}` and `{prompt}`) and `stages` |

## Stages

Each stage has an `id`, a `role` (`explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`), a `class`, an optional `gate`, and `next` rules:

- **`gate`**: `{docs: [file, …]}` for a document gate, or `{kind: commit}` for commit proposals. A stage without a gate (the built-in `pr` stage) declares none: the job is not stopped for approval there.
- **`class`**: `routine`, `critical`, `code` or `human-only`. Autonomy decides from it: `ask` approves nothing; `important` approves `routine` and `code` gates when no tripwire holds them; `critical` and `human-only` always wait. A stage with role `pr` is human-only whatever the manifest says.
- **`next`**: the command typed after approval, from a fixed vocabulary of conditions. `{prefix}`, `{key}`, `{prompt}`, `{commit}` and similar placeholders are filled in; `${…}` is left for the shell.
- **`loopUntil` / `then`**: repeat a stage until a condition (for example all tasks done), then move on.
- **`command`** (optional): the command that starts the stage (`"{prefix}design"`), the same text as the previous stage's `next`. xherd matches an approved gate's next command against it to know which stage a fresh session starts, and so which saved agent works there (`Flow::stage_for_command`). A stage without one, and a `next` that matches none (`{prefix}commit {commit}`, `{prefix}pr-apply`), stay in the same stage and agent. Which agent runs a stage is not in the pack: the user assigns it (Flows page, `xherd flow assign`).
- **`label`** (optional, 1–40 characters): the name the UI shows for the stage instead of one made from its `id`. The built-in `bugfix` flow keeps the id `implement` for its fix stage, which the shared prompts enter, and labels it `Fix`.

Rules that follow from the stages:

- **The task list** is `tasks.md` wherever it is gated: in a stage with role `tasks`, or in any stage whose `gate.docs` lists `tasks.md` (the bugfix `analysis` stage gates `spec.md` and `tasks.md` together). Its progress lines and the fix tasks a review or PR round adds never re-open that gate, and editing it after approval does.
- **Several commit stages**: a flow may have more than one stage with `{kind: commit}` (bugfix: `reproduce`, then `implement`). A commit proposal belongs to the job's current stage when that stage gates commits, else to the first one that does; that stage's `next` and `class` apply when it is approved.

## Command prompts

Each file in `commands/` becomes a runtime command (`/xherd:design`, …). Conventions that matter:

- Teach XAP as plain `xherd <call>` on one line in backticks, with concrete example values, never through a variable: Claude Code cannot check a command that uses a variable and asks every time. The worktree's `permissions.allow` lists the XAP subcommands.
- Every step starts a new session with no history. Each command reads `.xherd/spec/memory.md` first and rewrites it before its gate; with `memoryNote: required` (the built-in pack) the daemon refuses a gate or commit proposal without a valid one.
- Wrap nothing the daemon owns: it writes the `status:`/`evidence:`/`sha:` progress lines of `tasks.md` and `commits.md`, and makes approved commits itself.

`packs/xherd/FORMAT.md` documents the document formats (`tasks.md`, `commits.md`, `review.md`, `pr-review.md`, `memory.md`) that the daemon's parsers read.

## Installing

`xherd install` renders the pack's commands for each runtime (`{{prefix}}`, `{{args}}`, `{{dir}}`, `{{path:NAME}}` = where command NAME is installed for that runtime, `{{xap}}`, `{{format}}`) and writes them where the runtime reads commands. Format `command` writes each file as it is; `skill` (Codex) writes `<folder>/SKILL.md` with only `name` (the folder) and `description` in its frontmatter, plus `agents/openai.yaml` that keeps the skill from being used unless it is called by name. Hashes of what was written are kept, so a file you edited is left alone and the new version is written beside it with a `.xherd-new` suffix.

The daemon runs the same install at every start, natively and in the container, for every runtime that is switched on, so an upgrade needs no manual `xherd install`. It compares content (what this version renders, what is on disk, what xherd recorded): an outdated file you did not edit is replaced, and when everything is current nothing is written. A failure is a warning in the log, never a start failure. The same start also rewrites the hook file and the pre-push guard of every job that is not finished when they differ from what this version writes (a moved binary, new events or allowed calls); your own entries in a shared hook file are kept.

Two kinds of file need you, and Setup checks list them in a `pack-files` check (required for the default runtime):

- **Edited files.** The new version waits in `<file>.xherd-new`; until you decide, your jobs keep running your old prompt (for example, an edited `ship.md` from before the ticket summary existed still tells the job to update the ticket itself, which xherd now offers from the job page instead). The dialog offers *Use xherd's version* (replaces the file, your edits are lost) or *Keep mine* (deletes the `.xherd-new` and remembers that version, so later starts do not write it again; a different version in a later release writes a new `.xherd-new`).
- **Obsolete files.** A file xherd recorded that the pack no longer ships (a removed or renamed command). It is never deleted on its own: the dialog offers *Remove* (also removes a skill folder left empty) or *Keep* (xherd forgets the file and leaves it). `xherd install --remove-obsolete` removes them from the CLI. A file xherd never recorded is never touched.

The `pack` check also fails when a planned file is missing or out of date, not only when it is missing.

## Plugin packs

A pack other than `xherd` comes from a [flow plugin](plugins.md#workflow-plugins-kind-flow): its `data.flow.pack` names the folder that holds `xherd-flow.yaml`, `FORMAT.md` and `commands/`. The pack's `name` must be the plugin's name. Rules a plugin pack must follow, checked at preview, at load and before every write:

- **Install paths.** Each runtime's `commands.to` must land inside that runtime's command folder, in a sub-folder (or, for a skill, a skill folder) named for the pack, so it cannot overwrite the built-in pack's files, another pack's, or anything else: `${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md`, `${CODEX_HOME:-~/.codex}/skills/acme-sdd-{name}/SKILL.md`. A `to` that goes elsewhere, a `prefix` that does not belong to the pack (`/acme-sdd:`, `$acme-sdd-`) or a path another pack already owns (`path_taken`) is refused as `bad_manifest`.
- **Pack names.** `xherd` and `xherd-*` are reserved for the built-in pack.
- **`specDir: private`**, and no `schemas` key.
- **Every command the templates call has a prompt.** Each `{prefix}<name>` in a flow's `start`, a stage's `command`, `next[].run` and `autoApproved`, and in the `commands` map must match a file of `install.<runtime>.commands.from`; every `{{…}}` in a prompt must resolve. A pack that calls a missing command is refused.
- **Prompts** follow the conventions above; the pack may ship its own `FORMAT.md` (`{{format}}`).

A job picks its pack when it is created (`xherd new --pack`, `POST /jobs` `pack`, the New job form) and keeps it. Plugin packs are installed with the built-in one: at every daemon start and from `xherd install --pack <name>`, for every runtime that is on, with the same edited/obsolete file handling as above (the choices in `/install/review` and `/install/resolve` carry the `pack`). Setup checks show `pack:<pack>/<runtime>` and `pack-files:<pack>/<runtime>` for each plugin pack; they never block (only the built-in pack's check can be required).

## Limits today

The built-in pack ships `feature`, `bugfix` and `quick`. `quick` has one working stage (`implement`, labelled `Change`, a commit gate with class `code`) followed by `ship`, `pr-review` and `pr`, and no `review` stage: its prompts (`quick.md`, `commit.md`, `next.md`, `ship.md`, `pr-apply.md`) read the `Flow:` line of `.xherd/instructions.md` and send a quick job from its approved commit to `ship`, and a commit proposal made at the ship gate of a flow with no `review` stage is accepted (so a request for more code there becomes another commit). A quick job can be promoted to `feature` (`xherd promote`); that is built-in only, and plugin packs have no promotion. Any other pack comes from a flow plugin, which loads at daemon start (no live reload). Editing the installed command files works: `xherd install` keeps your edits (above). Fields not implemented yet (`views`, `extends`, `parse`, …) are rejected by the manifest parser instead of being silently ignored, so a plugin pack uses the xherd document formats. A new file in `packs/xherd/` must also be added to `xherd_flow::install::BUILTIN_FILES`.
