# Flow packs

A **flow pack** defines the stages agents go through, where they stop for approval, and the prompts that drive each step. A pack is a folder with `xherd-flow.yaml`, command prompts and a description of the document formats. The built-in `xherd` pack is in `packs/xherd/` and is embedded in the binary. Only the built-in pack can be loaded today (see [Limits today](#limits-today)); for running your own spec-driven method under xherd, planned, see [Add xherd support to your SDD](integrate-your-sdd.md); to teach xherd about a tool, see [Plugins](plugins.md).

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

Top-level fields:

| Field | Meaning |
|---|---|
| `name`, `version`, `protocol` | The pack, its version, and the XAP major version it speaks |
| `specDir` | Where the documents live (`private`: `.xherd/spec/` in the worktree, out of git) |
| `private` | Files of the spec folder that are xherd's working files, not documents |
| `install.<runtime>` | Per runtime: `prefix` (how a command is called: `/xherd:` for Claude Code, `/xherd/` for opencode, `$xherd-` for Codex), `commands` (`from` glob, `to` destination with `{name}`, `format` `command` or `skill`), `args` (how the runtime passes arguments), `slashEnters` (Enters needed to run a command from the completion list). An agent can use a runtime only if the pack has an entry for it |
| `commands` | Named commands xherd sends itself: `resume`, `handoff`, `status`, `prReview` |
| `flows` | Flows by name; each has `start` (the first command, with `{key}` and `{prompt}`) and `stages` |

## Stages

Each stage has an `id`, a `role` (`explore`, `spec`, `design`, `tasks`, `implement`, `review`, `ship`, `pr`), a `class`, an optional `gate`, and `next` rules:

- **`gate`**: `{docs: [file, …]}` for a document gate, or `{kind: commit}` for commit proposals. A stage without a gate (the built-in `pr` stage) declares none: the agent is not stopped for approval there.
- **`class`**: `routine`, `critical`, `code` or `human-only`. Autonomy decides from it: `ask` approves nothing; `important` approves `routine` and `code` gates when no tripwire holds them; `critical` and `human-only` always wait. A stage with role `pr` is human-only whatever the manifest says.
- **`next`**: the command typed after approval, from a fixed vocabulary of conditions. `{prefix}`, `{key}`, `{prompt}`, `{commit}` and similar placeholders are filled in; `${…}` is left for the shell.
- **`loopUntil` / `then`**: repeat a stage until a condition (for example all tasks done), then move on.

## Command prompts

Each file in `commands/` becomes a runtime command (`/xherd:design`, …). Conventions that matter:

- Teach XAP as plain `xherd <call>` on one line in backticks, with concrete example values, never through a variable: Claude Code cannot check a command that uses a variable and asks every time. The worktree's `permissions.allow` lists the XAP subcommands.
- Every step starts a new session with no history. Each command reads `.xherd/spec/memory.md` first and rewrites it before its gate; the daemon refuses a gate or commit proposal without a valid one.
- Wrap nothing the daemon owns: it writes the `status:`/`evidence:`/`sha:` progress lines of `tasks.md` and `commits.md`, and makes approved commits itself.

`packs/xherd/FORMAT.md` documents the document formats (`tasks.md`, `commits.md`, `review.md`, `pr-review.md`, `memory.md`) that the daemon's parsers read.

## Installing

`xherd install` renders the pack's commands for each runtime (`{{prefix}}`, `{{args}}`, `{{dir}}`, `{{path:NAME}}` = where command NAME is installed for that runtime, `{{xap}}`, `{{format}}`) and writes them where the runtime reads commands. Format `command` writes each file as it is; `skill` (Codex) writes `<folder>/SKILL.md` with only `name` (the folder) and `description` in its frontmatter, plus `agents/openai.yaml` that keeps the skill from being used unless it is called by name. Hashes of what was written are kept, so a file you edited is left alone and the new version is written beside it with a `.xherd-new` suffix. In the container this runs at every daemon start. After an update, look for `.xherd-new` files next to commands you edited: until you merge them, your agents keep running your old prompt (for example, an edited `ship.md` from before the ticket summary existed still tells the agent to update the ticket itself, which xherd now offers from the agent page instead).

## Limits today

Only the built-in `xherd` pack and its `feature` flow ship, and xherd cannot load another pack yet (`xherd install --pack` accepts only `xherd`). Editing the installed command files works: `xherd install` keeps your edits (above). Fields not implemented yet (`views`, `extends`, `parse`, …) are rejected by the manifest parser instead of being silently ignored. A new file in `packs/xherd/` must also be added to `xherd_flow::install::BUILTIN_FILES`.
