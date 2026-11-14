<script lang="ts">
  // Settings → Open in (R-706, R-707): the terminal on this computer (an
  // `xherd-term://` link handler installed once), your own app links, and
  // which entries the agent's Open in menu shows. Every entry is a link the
  // browser opens: this page can never make your computer run a command.
  import { get, post, put } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { setup, loadConfig } from '../lib/setup.svelte';
  import { agoText } from '../lib/format';
  import { badLinkUrl, handlerState, installText, linkId } from '../lib/openIn';
  import type { ConfigView, OpenInConfig, OpenInView } from '../lib/types';
  import Panel from './Panel.svelte';
  import Callout from './Callout.svelte';
  import SaveBar from './SaveBar.svelte';

  type Link = { id: string; label: string; url: string };
  // A new row's id follows its name; a saved one keeps its id (hidden marks use it).
  type Row = Link & { fresh?: boolean };

  // Every terminal the handler knows (config::TERMINAL_APPS). The list shows
  // the ones the handler found on its computer, or all of them before that.
  const TERMINALS: Record<string, string> = {
    terminal: 'Terminal (macOS)', iterm: 'iTerm (macOS)', ghostty: 'Ghostty', konsole: 'Konsole', 'gnome-terminal': 'GNOME Terminal',
    kitty: 'kitty', alacritty: 'Alacritty', wezterm: 'WezTerm', 'xfce4-terminal': 'Xfce Terminal', foot: 'foot', xterm: 'xterm',
    'xdg-terminal-exec': 'xdg-terminal-exec (Linux)', 'x-terminal-emulator': 'x-terminal-emulator (Debian alternatives)',
  };

  let view = $state<OpenInView | null>(null);
  let hidden = $state<string[]>([]);
  let terminal = $state('');
  let rows = $state<Row[]>([]);
  let saving = $state(false);
  let installing = $state(false);
  let output = $state<{ ok: boolean; text: string } | null>(null);
  let copied = $state('');

  const saved = $derived((setup.config?.config.openIn ?? {}) as OpenInConfig);
  const dropped = $derived(saved.droppedCommands ?? []);
  function fill() {
    hidden = [...(saved.hidden ?? [])];
    terminal = saved.terminal ?? '';
    rows = (saved.links ?? []).map((l) => ({ ...l }));
  }
  let filled = false;
  $effect(() => { if (setup.config && !filled) { filled = true; fill(); } });
  $effect(() => { if (!setup.config) loadConfig().catch((e) => showError('Could not load settings', e)); });
  async function refresh() {
    try { view = await get<OpenInView>('/open-in'); } catch (e) { showError('Could not read the Open-in entries', e); }
  }
  $effect(() => { refresh(); });

  const links = $derived<Link[]>(rows.map((r) => ({ id: r.id, label: r.label.trim(), url: r.url.trim() })));
  const urlError = (u: string) => (u.trim() ? badLinkUrl(u) : null);
  const bad = $derived(links.some((l) => !l.label || !l.url || badLinkUrl(l.url)));
  // droppedCommands goes back as saved: the daemon lets it only shrink, and Dismiss clears it.
  const body = $derived<OpenInConfig>({ hidden, ...(terminal ? { terminal } : {}), links, droppedCommands: dropped });
  const norm = (o: OpenInConfig) => JSON.stringify({ h: [...(o.hidden ?? [])].sort(), t: o.terminal ?? '', l: o.links ?? [] });
  const dirty = $derived(norm(body) !== norm(saved));

  const record = $derived(view?.handler.installed ?? null);
  const hstate = $derived(view ? handlerState(view) : null);
  const how = $derived(view ? installText(location.origin, view.native) : null);
  const found = $derived((record?.terminals ?? []).filter((t) => t in TERMINALS));
  const termOptions = $derived.by(() => {
    const ids = found.length ? [...found] : Object.keys(TERMINALS);
    if (terminal && !ids.includes(terminal)) ids.push(terminal);
    return ids.map((id) => [id, TERMINALS[id] ?? id] as const);
  });

  const entries = $derived([
    ...(view?.links ?? []).map((l) => ({ id: l.id, label: l.label, where: 'editor link' })),
    { id: 'terminal', label: 'Terminal', where: hstate === 'missing' || !hstate ? 'shown once the terminal is set up on this computer' : 'your terminal app' },
    ...links.filter((l) => l.label).map((l) => ({ id: `link:${l.id}`, label: l.label, where: 'your link' })),
  ]);
  function toggle(id: string) {
    hidden = hidden.includes(id) ? hidden.filter((h) => h !== id) : [...hidden, id];
  }
  function addLink() {
    const r: Row = { id: '', label: '', url: '', fresh: true };
    r.id = linkId('link', rows.map((x) => x.id));
    rows = [...rows, r];
  }

  async function saveOpenIn(o: OpenInConfig) {
    setup.config = await put<ConfigView>('/config', { openIn: o });
  }
  async function save() {
    saving = true;
    try {
      await saveOpenIn(body);
      fill();
    } catch (e) {
      showError('Could not save the Open-in settings', e);
    } finally {
      saving = false;
    }
  }
  // Dismiss saves at once, with what is saved: edits on the page stay unsaved.
  async function dismiss() {
    try { await saveOpenIn({ ...saved, droppedCommands: [] }); } catch (e) { showError('Could not dismiss the notice', e); }
  }

  async function install(uninstall: boolean) {
    installing = true;
    output = null;
    try {
      const r = await post<{ ok: boolean; output: string }>('/open-in/terminal-handler/install', uninstall ? { uninstall: true } : {});
      output = { ok: r.ok, text: r.output };
    } catch (e) {
      showError(uninstall ? 'Could not remove the terminal handler' : 'Could not install the terminal handler', e);
    } finally {
      installing = false;
      refresh();
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = text;
      setTimeout(() => { if (copied === text) copied = ''; }, 2000);
    } catch (e) {
      showError('Could not copy (select the command and copy it by hand)', e);
    }
  }
</script>

<div class="secs">
  <Panel title="Terminal on this computer" hint="Open in → Terminal opens your own terminal app in the agent's worktree. It needs a one-time setup on this computer and keeps nothing running.">
    {#if !view}
      <p class="muted">Loading…</p>
    {:else}
      {#if hstate === 'missing'}
        <Callout tone="info">Not set up on this computer yet.</Callout>
      {:else if record && view.handler.rootChanged}
        <Callout tone="need">Set up for another worktrees folder (<span class="mono">{record.root}</span>): install again.</Callout>
      {:else if record && hstate === 'update'}
        <Callout tone="need">A newer version of the handler is available: install again.</Callout>
      {:else if record}
        <Callout tone="ok">Set up ({record.os === 'mac' ? 'macOS' : 'Linux'}, {agoText(record.at)}).</Callout>
      {/if}

      {#if how?.native}
        <div class="row">
          <button class="btn" disabled={installing} onclick={() => install(false)}>{installing ? 'Working…' : hstate === 'missing' ? 'Install' : hstate === 'update' ? 'Update' : 'Install again'}</button>
          {#if hstate !== 'missing'}<button class="btn ghost" disabled={installing} onclick={() => install(true)}>Remove</button>{/if}
        </div>
        <p class="hint">xherd runs the installer as you, on this computer. It adds a small script and registers the <span class="mono">xherd-term://</span> link type.</p>
        {#if output}
          {#if !output.ok}<p class="err">The installer failed:</p>{/if}
          {#if output.text}<pre class="out mono">{output.text}</pre>{/if}
        {/if}
      {:else if how}
        <p class="hint">Run this in a terminal on this computer (not in the container), then Check again:</p>
        <div class="cmdline">
          <code class="mono">{how.install}</code>
          <button class="btn sm" onclick={() => copy(how.install)}>{copied === how.install ? 'Copied' : 'Copy'}</button>
        </div>
        <p class="hint">To remove it:</p>
        <div class="cmdline">
          <code class="mono">{how.uninstall}</code>
          <button class="btn sm" onclick={() => copy(how.uninstall)}>{copied === how.uninstall ? 'Copied' : 'Copy'}</button>
        </div>
        <button class="btn sm" onclick={refresh}>Check again</button>
      {/if}

      <label class="f">Terminal app
        <select bind:value={terminal}>
          <option value="">Default for this computer{found.length ? ` (${TERMINALS[found[0]]})` : ''}</option>
          {#each termOptions as [v, l] (v)}<option value={v}>{l}{found.length && !found.includes(v) ? ' (not found)' : ''}</option>{/each}
        </select>
        <span class="hint">{found.length ? 'Found on this computer by the installer.' : 'Set up the terminal to list the ones installed here.'} One that is not installed falls back to the default.</span></label>
      <p class="hint">Your browser asks before each launch. In Chrome, "Always allow" removes the question for this site.</p>
    {/if}
  </Panel>

  {#if dropped.length}
    <Callout tone="info">Your commands ({dropped.join(', ')}) were removed: xherd no longer runs commands on your computer. If the app has a link type (like <span class="mono">idea://</span>), add it under Your links.
      <button class="btn sm" onclick={dismiss}>Dismiss</button></Callout>
  {/if}

  <Panel title="Your links" hint="Apps on your computer that open by a link, like idea://open?file={'{dir}'}. {'{dir}'} is the worktree, {'{agent}'} the agent's name.">
    {#each rows as r, i (i)}
      {@const err = urlError(r.url)}
      <div class="lnk">
        <label class="f">Name
          <input bind:value={r.label} oninput={() => { if (r.fresh) r.id = linkId(r.label, rows.filter((x) => x !== r).map((x) => x.id)); }} placeholder="IntelliJ IDEA" /></label>
        <label class="f">URL
          <input class="mono" bind:value={r.url} placeholder={'idea://open?file={dir}'} aria-invalid={!!err} /></label>
        <button class="btn ghost sm" aria-label={`Remove ${r.label || 'link'}`} onclick={() => (rows = rows.filter((x) => x !== r))}>Remove</button>
        {#if err}<p class="err">The URL {err}.</p>{/if}
      </div>
    {/each}
    {#if bad}<p class="err">Each link needs a name and an app URL.</p>{/if}
    <p class="hint">Only links to apps: web pages, files and browser links are refused.</p>
    {#snippet foot()}
      <button class="btn" onclick={addLink}>Add a link</button>
    {/snippet}
  </Panel>

  <Panel title="In the menu" hint="What the agent page's Open worktree in… shows." flush>
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
  .row { display: flex; gap: 8px; margin: 10px 0 4px; }
  .cmdline { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; }
  .cmdline code { flex: 1; min-width: 0; padding: 6px 8px; background: var(--well); border: 1px solid var(--fence-soft); border-radius: var(--r-ctl); overflow-x: auto; white-space: nowrap; }
  .out { max-height: 220px; overflow: auto; padding: 8px; background: var(--well); border: 1px solid var(--fence-soft); border-radius: var(--r-ctl); white-space: pre-wrap; }
  .lnk { display: grid; grid-template-columns: 1fr 2fr auto; gap: 8px; align-items: end; margin-bottom: 10px; }
  .lnk .err { grid-column: 1 / -1; margin: 0; }
  @media (max-width: 760px) { .lnk { grid-template-columns: 1fr; } }
</style>
