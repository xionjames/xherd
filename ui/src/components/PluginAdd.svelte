<script lang="ts">
  // The Add plugin dialog (R14): a git URL (and optionally a ref), a preview of
  // what it holds and runs, then Install, which sends back the previewed digest.
  // Nothing from the repository runs before the user installs.
  import Dialog from './Dialog.svelte';
  import Callout from './Callout.svelte';
  import { pluginPreview, pluginInstall, pluginCatalog } from '../lib/api';
  import type { CatalogEntry, InstalledPlugin, PluginCatalog, PluginPreview } from '../lib/types';

  let { onclose, oninstalled }: { onclose: () => void; oninstalled: () => void } = $props();

  let url = $state('');
  let gitRef = $state('');
  let path = $state<string | undefined>(undefined);
  let preview = $state<PluginPreview | null>(null);
  let done = $state<InstalledPlugin | null>(null);
  let busy = $state(false);
  let error = $state('');

  // The catalogue: entries come from other people's repositories, so their
  // text is shown as plain text only.
  let query = $state('');
  let catalog = $state<PluginCatalog | null>(null);
  let listing = $state(false);
  let catalogError = $state('');
  let seq = 0;
  async function loadCatalog(q: string, refresh = false) {
    const mine = ++seq;
    listing = true;
    catalogError = '';
    try {
      const c = await pluginCatalog(q.trim(), refresh);
      if (mine === seq) catalog = c;
    } catch (e) {
      if (mine === seq) catalogError = message(e);
    } finally {
      if (mine === seq) listing = false;
    }
  }
  // First load at once, then 250 ms after the last keystroke.
  let first = true;
  $effect(() => {
    const q = query;
    const t = setTimeout(() => loadCatalog(q), first ? 0 : 250);
    first = false;
    return () => clearTimeout(t);
  });
  function choose(e: CatalogEntry) {
    url = e.url;
    gitRef = e.ref ?? '';
    preview = null;
    done = null;
    look(e.path);
  }
  const failed = $derived((catalog?.sources ?? []).filter((s) => s.error));
  const skipped = $derived((catalog?.sources ?? []).reduce((n, s) => n + (s.skipped ?? 0), 0));

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
  // Any change to the source makes the previewed digest stale.
  function edited() {
    preview = null;
    path = undefined;
    error = '';
  }
  async function look(p?: string) {
    if (!url.trim()) return;
    busy = true;
    error = '';
    try {
      path = p;
      preview = await pluginPreview(url.trim(), gitRef.trim() || undefined, p);
    } catch (e) {
      preview = null;
      error = message(e);
    } finally {
      busy = false;
    }
  }
  async function install() {
    if (!preview?.digest) return;
    busy = true;
    error = '';
    try {
      done = await pluginInstall(preview.url, preview.digest, preview.ref ?? (gitRef.trim() || undefined), preview.path ?? path);
      oninstalled();
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
    }
  }
  const size = (n?: number) => (n === undefined ? '' : n < 1024 ? `${n} B` : `${Math.round(n / 1024)} KiB`);
  const ready = $derived(!!preview?.digest && preview.compatible !== false);
</script>

<Dialog title="Add a plugin" {onclose}>
  {#if done}
    <Callout tone="ok">Installed <b>{done.name}</b> {done.version} from <span class="mono">{done.url}@{done.sha.slice(0, 7)}</span>.</Callout>
    <p>Restart needed: the plugin loads when xherd restarts. Running agents keep going.</p>
  {:else}
    <section class="stack-in" aria-label="Catalogue">
      <label class="f">Find a plugin
        <input type="search" bind:value={query} placeholder="Search the catalogue" autocomplete="off" spellcheck="false" /></label>
      {#if catalogError}<Callout tone="bad"><span class="wrap-any">{catalogError}</span></Callout>{/if}
      {#if catalog}
        {#if catalog.entries.length}
          <ul class="plain catalog">
            {#each catalog.entries as e (e.source + e.url + (e.path ?? ''))}
              <li><button class="btn sm" disabled={busy} onclick={() => choose(e)}>{e.name}</button>
                <span class="hint">{e.owner}{e.kind ? ` · ${e.kind}` : ''}{e.stars !== undefined ? ` · ${e.stars} stars` : ''}{e.installed.length ? ' · installed' : ''}</span>
                {#if e.description}<span class="desc wrap-any">{e.description}</span>{/if}</li>
            {/each}
          </ul>
        {:else if !listing}
          <p class="hint">Nothing in the catalogue{query.trim() ? ' matches' : ' yet'}. You can still paste a Git URL below.</p>
        {/if}
        {#each failed as s (s.source)}<span class="hint wrap-any">Source {s.source} failed: {s.error}</span>{/each}
        {#if skipped}<span class="hint">{skipped} catalogue {skipped === 1 ? 'entry was' : 'entries were'} skipped because {skipped === 1 ? 'it was' : 'they were'} not valid.</span>{/if}
      {/if}
      <div class="row"><button class="btn sm" type="button" disabled={listing} onclick={() => loadCatalog(query, true)}>{listing ? 'Searching…' : 'Refresh'}</button></div>
    </section>

    <form class="stack-in" onsubmit={(e) => { e.preventDefault(); look(); }}>
      <label class="f">Git URL
        <input class="mono" bind:value={url} oninput={edited} placeholder="https://github.com/owner/plugin" autocomplete="off" spellcheck="false" />
        <span class="hint">A repository with an <span class="mono">xherd-plugin.json</span>, or a GitHub <span class="mono">tree/&lt;ref&gt;/&lt;folder&gt;</span> link. Private repositories use your Git access.</span></label>
      <label class="f">Branch, tag or commit (optional)
        <input class="mono" bind:value={gitRef} oninput={edited} placeholder="the default branch" autocomplete="off" spellcheck="false" /></label>
      <div class="row"><button class="btn" type="submit" disabled={busy || !url.trim()}>{busy && !preview ? 'Looking…' : 'Preview'}</button></div>
    </form>

    {#if error}<Callout tone="bad"><span class="wrap-any">{error}</span></Callout>{/if}

    {#if preview?.choices?.length}
      <p>This repository holds several plugins. Choose one.</p>
      <ul class="plain">
        {#each preview.choices as c (c.path)}
          <li class="row"><button class="btn sm" disabled={busy || !!c.error} onclick={() => look(c.path)}>{c.name}</button>
            <span class="mono hint">{c.path}</span>{#if c.kind}<span class="hint">{c.kind}</span>{/if}
            {#if c.error}<span class="err">{c.error}</span>{/if}</li>
        {/each}
      </ul>
    {:else if preview?.name}
      <div class="preview">
        <h3>{preview.label || preview.name} <span class="hint">{preview.version} · {preview.kind}</span></h3>
        {#if preview.description}<p>{preview.description}</p>{/if}
        <dl class="kv">
          <dt>Source</dt><dd class="mono wrap-any">{preview.url}@{preview.sha.slice(0, 7)}{preview.path ? ` · ${preview.path}` : ''}</dd>
          <dt>Size</dt><dd>{preview.files} files, {size(preview.bytes)}</dd>
          {#if preview.replaces}<dt>Replaces</dt><dd>{preview.replaces === 'builtin' ? 'the built-in plugin of this name' : 'the installed plugin of this name'}</dd>{/if}
        </dl>
        {#if preview.warning}<Callout tone="need"><span class="wrap-any">{preview.warning}</span></Callout>{/if}
        {#if preview.compatible === false}<Callout tone="bad">Needs xherd {preview.minXherdVersion} or newer.</Callout>{/if}
        {#if preview.runs?.length}
          <span class="lbl">It will run</span>
          <ul class="plain">
            {#each preview.runs as r, i (i)}<li><span class="hint">{r.what}</span> <code class="mono wrap-any">{r.argv.join(' ')}</code></li>{/each}
          </ul>
        {/if}
        {#if preview.settings?.length}
          <span class="lbl">Settings it asks for</span>
          <p>{preview.settings.map((s) => s.label || s.key).join(', ')}</p>
        {/if}
        {#if preview.flow}
          {@const fl = preview.flow}
          {#if fl.inUseBy}<Callout tone="need">A job still uses the pack <span class="mono">{fl.pack}</span>: <span class="wrap-any">{fl.inUseBy}</span>. It cannot be updated or removed until that job is done.</Callout>{/if}
          <span class="lbl">Flow pack <span class="mono">{fl.pack}</span></span>
          <ul class="plain">
            {#each fl.flows as f (f.flow)}
              <li><b>{f.flow}</b> <span class="hint">{f.stages.map((s) => s.id + (s.gate ? ' (gate)' : '')).join(' → ')}</span></li>
            {/each}
          </ul>
          {#if fl.memoryNote === 'off'}<p class="hint">Jobs of this pack need no <span class="mono">memory.md</span>.</p>{/if}
          {#if fl.agents.length}
            <span class="lbl">Agents</span>
            <ul class="plain">
              {#each fl.agents as a (a.savedName)}
                <li><span class="mono">{a.savedName}</span> <span class="hint">{a.state}</span>
                  {#if a.risky.length}<span class="hint">risky: {a.risky.join(', ')}</span>{/if}
                  {#each a.issues as i (i)}<span class="err">{i}</span>{/each}</li>
              {/each}
            </ul>
          {/if}
          {#if fl.archived.length}<p class="hint">Archived (the new version drops them): <span class="mono">{fl.archived.join(', ')}</span></p>{/if}
          {#if fl.assignmentChanges.length}
            <span class="lbl">Assignment changes</span>
            <ul class="plain">
              {#each fl.assignmentChanges as c (c.flow + c.stage)}
                <li><span class="mono">{c.flow} / {c.stage}</span> <span class="hint">{c.from} → {c.to ?? 'none'}</span></li>
              {/each}
            </ul>
          {/if}
          {#each fl.files as r (r.runtime)}
            <span class="lbl">Files for {r.runtime}</span>
            <ul class="plain">{#each r.paths as p (p)}<li class="mono wrap-any">{p}</li>{/each}</ul>
          {/each}
        {/if}
      </div>
    {/if}
  {/if}
  {#snippet actions()}
    {#if done}
      <button class="btn primary" onclick={onclose}>Close</button>
    {:else}
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy || !ready} onclick={install}>{busy && preview ? 'Installing…' : 'Install'}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .stack-in, .preview { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  .preview { margin-top: 12px; }
  .preview h3, .preview p { margin: 0; }
  .catalog { max-height: 220px; overflow-y: auto; }
  .catalog li { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 8px; }
  .desc { flex-basis: 100%; }
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
</style>
