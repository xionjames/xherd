<script lang="ts">
  // Settings → Open in (R-706, R-707): which entries the agent's Open in menu
  // shows, the terminal app, and your own commands. Your commands run on this
  // computer through `xherd companion`, which asks you there before running
  // one it has not seen (or one you changed), so this page alone can never
  // make your computer run something new.
  import { get, put } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { setup, loadConfig } from '../lib/setup.svelte';
  import { line, words } from '../lib/argv';
  import type { ConfigView } from '../lib/types';
  import Panel from './Panel.svelte';
  import Callout from './Callout.svelte';
  import SaveBar from './SaveBar.svelte';

  type Command = { id: string; label: string; argv: string[] };
  type OpenIn = { hidden?: string[]; terminal?: string; commands?: Command[] };
  // A new row's id follows its name; a saved one keeps its id (hidden marks use it).
  type Row = { id: string; label: string; cmd: string; fresh?: boolean };
  type View = { links: { id: string; label: string }[]; companion: { apps: string[]; features: string[] } | null };

  const TERMINALS = [['terminal', 'Terminal (macOS)'], ['iterm', 'iTerm (macOS)'], ['ghostty', 'Ghostty (macOS)'], ['x-terminal-emulator', 'x-terminal-emulator (Linux)']];

  let view = $state<View | null>(null);
  let hidden = $state<string[]>([]);
  let terminal = $state('');
  let rows = $state<Row[]>([]);
  let saving = $state(false);

  const saved = $derived(((setup.config?.config.openIn ?? {}) as OpenIn));
  function fill() {
    hidden = [...(saved.hidden ?? [])];
    terminal = saved.terminal ?? '';
    rows = (saved.commands ?? []).map((c) => ({ id: c.id, label: c.label, cmd: line(c.argv) }));
  }
  let filled = false;
  $effect(() => { if (setup.config && !filled) { filled = true; fill(); } });
  $effect(() => { if (!setup.config) loadConfig().catch((e) => showError('Could not load settings', e)); });
  async function refresh() {
    try { view = await get<View>('/open-in'); } catch (e) { showError('Could not read the Open-in entries', e); }
  }
  $effect(() => { refresh(); });

  const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'command';
  function idFor(label: string, self: Row) {
    const base = slug(label);
    let id = base;
    for (let n = 2; rows.some((r) => r !== self && r.id === id); n++) id = `${base}-${n}`;
    return id;
  }
  const commands = $derived<Command[]>(rows.map((r) => ({ id: r.id, label: r.label.trim(), argv: words(r.cmd) })));
  const bad = $derived(commands.some((c) => !c.label || !c.argv[0]));
  const body = $derived<OpenIn>({ hidden, ...(terminal ? { terminal } : {}), commands });
  const norm = (o: OpenIn) => JSON.stringify({ h: [...(o.hidden ?? [])].sort(), t: o.terminal ?? '', c: o.commands ?? [] });
  const dirty = $derived(norm(body) !== norm(saved));

  const entries = $derived([
    ...(view?.links ?? []).map((l) => ({ id: l.id, label: l.label, where: 'link' })),
    { id: 'terminal', label: 'Terminal (shell in the worktree)', where: 'companion' },
    ...(view?.companion?.apps ?? []).map((a) => ({ id: `companion:${a}`, label: a, where: 'companion app' })),
    ...commands.filter((c) => c.label).map((c) => ({ id: `cmd:${c.id}`, label: c.label, where: 'your command' })),
  ]);
  function toggle(id: string) {
    hidden = hidden.includes(id) ? hidden.filter((h) => h !== id) : [...hidden, id];
  }

  async function save() {
    saving = true;
    try {
      setup.config = await put<ConfigView>('/config', { openIn: body });
      fill();
    } catch (e) {
      showError('Could not save the Open-in settings', e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="secs">
  <Panel title="Companion" hint="Opens the terminal and runs commands on this computer.">
    {#if !view}
      <p class="muted">Loading…</p>
    {:else if view.companion}
      <Callout tone="ok">Connected{view.companion.apps.length ? `, offering ${view.companion.apps.join(', ')}` : ''}.</Callout>
      {#if !view.companion.features.includes('terminal')}
        <Callout tone="info">This companion is older than the terminal and your commands: update xherd on this computer and start it again.</Callout>
      {/if}
    {:else}
      <Callout tone="info">Not running. In a terminal on this computer, run <span class="mono">xherd companion</span> and leave it open.</Callout>
    {/if}
    <p class="hint">Links (VS Code, Cursor, Zed) need no companion. Your own commands do, and the first time each one runs, or after you change it, the companion asks you in its window before running it. Apps listed in <span class="mono">~/.config/xherd/open.json</span> on this computer run without asking.</p>
    <button class="btn sm" onclick={refresh}>Check again</button>
  </Panel>

  <Panel title="Terminal" hint="Open in → Terminal opens a shell in the agent's worktree (inside the container in Docker mode).">
    <label class="f">Terminal app
      <select bind:value={terminal}>
        <option value="">Default for this computer</option>
        {#each TERMINALS as [v, l] (v)}<option value={v}>{l}</option>{/each}
      </select>
      <span class="hint">The command is built into xherd (<span class="mono">xherd shell &lt;agent&gt;</span>), so it never needs confirming.</span></label>
  </Panel>

  <Panel title="Your commands" hint="Programs to open the worktree with. {'{dir}'} is the worktree, {'{agent}'} the agent's name.">
    {#each rows as r, i (i)}
      <div class="cmd">
        <label class="f">Name
          <input bind:value={r.label} oninput={() => { if (r.fresh) r.id = idFor(r.label, r); }} placeholder="IntelliJ IDEA" /></label>
        <label class="f">Command
          <input class="mono" bind:value={r.cmd} placeholder={'idea {dir}'} /></label>
        <button class="btn ghost sm" aria-label={`Remove ${r.label || 'command'}`} onclick={() => (rows = rows.filter((x) => x !== r))}>Remove</button>
      </div>
    {/each}
    {#if bad}<p class="err">Each command needs a name and a program.</p>{/if}
    <p class="hint">No shell: quotes keep spaces, nothing else is interpreted. Use <span class="mono">open -a "App Name" {'{dir}'}</span> for a Mac app.</p>
    {#snippet foot()}
      <button class="btn" onclick={() => { const r: Row = { id: '', label: '', cmd: '', fresh: true }; r.id = idFor('command', r); rows = [...rows, r]; }}>Add a command</button>
    {/snippet}
  </Panel>

  <Panel title="In the menu" hint="What the agent page's Open in… shows. Companion entries also need the companion running." flush>
    <ul class="list">
      {#each entries as e (e.id)}
        <li>
          <div class="tog">
            <button class="sw" class:on={!hidden.includes(e.id)} role="switch" aria-checked={!hidden.includes(e.id)} aria-label={`Show ${e.label}`}
              onclick={() => toggle(e.id)}></button>
            <div><b style="font-weight:500">{e.label}</b><div class="hint">{e.where}</div></div>
          </div>
        </li>
      {/each}
    </ul>
  </Panel>
</div>
<SaveBar dirty={dirty && !bad} busy={saving} onsave={save} ondiscard={fill} />

<style>
  .cmd { display: grid; grid-template-columns: 1fr 2fr auto; gap: 8px; align-items: end; margin-bottom: 10px; }
  @media (max-width: 760px) { .cmd { grid-template-columns: 1fr; } }
</style>
