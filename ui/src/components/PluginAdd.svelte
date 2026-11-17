<script lang="ts">
  // The Add plugin dialog (R14): a git URL (and optionally a ref), a preview of
  // what it holds and runs, then Install, which sends back the previewed digest.
  // Nothing from the repository runs before the user installs.
  import Dialog from './Dialog.svelte';
  import Callout from './Callout.svelte';
  import { pluginPreview, pluginInstall } from '../lib/api';
  import type { InstalledPlugin, PluginPreview } from '../lib/types';

  let { onclose, oninstalled }: { onclose: () => void; oninstalled: () => void } = $props();

  let url = $state('');
  let gitRef = $state('');
  let path = $state<string | undefined>(undefined);
  let preview = $state<PluginPreview | null>(null);
  let done = $state<InstalledPlugin | null>(null);
  let busy = $state(false);
  let error = $state('');

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
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
</style>
