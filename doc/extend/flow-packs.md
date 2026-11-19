# Flow packs

A **flow pack** defines the stages jobs go through, where they stop for approval, and the prompts that drive each step. A pack is a folder with `xherd-flow.yaml`, command prompts and a description of the document formats. The built-in `xherd` pack is in `packs/xherd/` and is embedded in the binary. Only the built-in pack can be loaded today (see [Limits today](#limits-today)); for running your own spec-driven method under xherd, planned, see [Add xherd support to your SDD](integrate-your-sdd.md); to teach xherd about a tool, see [Plugins](plugins.md).

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
| `install.<runtime>` | Per runtime: `prefix` (how a command is called: `/xherd:` for Claude Code, `/xherd/` for opencode, `$xherd-` for Codex), `commands` (`from` glob, `to` destination with `{name}`, `format` `command` or `skill`), `args` (how the runtime passes arguments), `slashEnters` (Enters needed to run a command from the completion list). A job can use a runtime only if the pack has an entry for it |
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
- Every step starts a new session with no history. Each command reads `.xherd/spec/memory.md` first and rewrites it before its gate; the daemon refuses a gate or commit proposal without a valid one.
- Wrap nothing the daemon owns: it writes the `status:`/`evidence:`/`sha:` progress lines of `tasks.md` and `commits.md`, and makes approved commits itself.

`packs/xherd/FORMAT.md` documents the document formats (`tasks.md`, `commits.md`, `review.md`, `pr-review.md`, `memory.md`) that the daemon's parsers read.

## Installing

`xherd install` renders the pack's commands for each runtime (`{{prefix}}`, `{{args}}`, `{{dir}}`, `{{path:NAME}}` = where command NAME is installed for that runtime, `{{xap}}`, `{{format}}`) and writes them where the runtime reads commands. Format `command` writes each file as it is; `skill` (Codex) writes `<folder>/SKILL.md` with only `name` (the folder) and `description` in its frontmatter, plus `agents/openai.yaml` that keeps the skill from being used unless it is called by name. Hashes of what was written are kept, so a file you edited is left alone and the new version is written beside it with a `.xherd-new` suffix. In the container this runs at every daemon start. After an update, look for `.xherd-new` files next to commands you edited: until you merge them, your jobs keep running your old prompt (for example, an edited `ship.md` from before the ticket summary existed still tells the job to update the ticket itself, which xherd now offers from the job page instead).

## Limits today

Only the built-in `xherd` pack and its `feature` and `bugfix` flows ship, and xherd cannot load another pack yet (`xherd install --pack` accepts only `xherd`). Editing the installed command files works: `xherd install` keeps your edits (above). Fields not implemented yet (`views`, `extends`, `parse`, …) are rejected by the manifest parser instead of being silently ignored. A new file in `packs/xherd/` must also be added to `xherd_flow::install::BUILTIN_FILES`.
