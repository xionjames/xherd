<script lang="ts">
  // Settings → Toolchains (R-653): a default version per tool, used by a
  // worktree without a version file, and installing a version (container only,
  // in a live terminal). A default applies to agents started after the daemon
  // restarts, like the other saved settings.
  import { get, put, post } from '../lib/api';
  import { loadConfig } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';

  interface Tool { plugin: string; pluginLabel: string; tool: string; default: string | null; canInstall: boolean }
  let tools = $state<Tool[]>([]);
  let sandbox = $state(false);
  let versions = $state<Record<string, string>>({});
  let installing = $state<string | null>(null);

  async function load() {
    try {
      const r = await get<{ tools: Tool[]; sandbox: boolean }>('/toolchains');
      tools = r.tools;
      sandbox = r.sandbox;
      versions = Object.fromEntries(r.tools.map((t) => [t.tool, t.default ?? '']));
    } catch (e) { showError('Could not load toolchains', e); }
  }
  $effect(() => { load(); });

  async function saveDefault(t: Tool, value: string) {
    const toolchainDefaults = Object.fromEntries(tools.map((x) => [x.tool, x.tool === t.tool ? value : (x.default ?? '')]));
    try {
      await put('/config', { toolchainDefaults });
      await load();
      await loadConfig();
    } catch (e) { showError(`Could not save the default for ${t.tool}`, e); }
  }
  async function install(t: Tool) {
    try {
      await post('/toolchains/install', { tool: t.tool, version: versions[t.tool].trim() });
      installing = t.tool;
    } catch (e) { showError(`Could not install ${t.tool}`, e); }
  }
  const groups = $derived([...new Set(tools.map((t) => t.pluginLabel))]);
</script>

<p class="muted" style="max-width:72ch">A worktree's own version files (<span class="mono">.nvmrc</span>, <span class="mono">.sdkmanrc</span>, <span class="mono">go.mod</span>…) always win.
  The default is what an agent gets for a tool its repository does not pin. Leave it empty for no default.</p>
{#if !sandbox}
  <p class="hint">xherd runs natively: it never installs toolchains on your computer, so only the defaults can be set here. Run xherd in Docker to install versions.</p>
{/if}

{#each groups as g (g)}
  <section class="sec">
    <div class="sh"><h2>{g}</h2></div>
    <div class="bd">
      {#each tools.filter((t) => t.pluginLabel === g) as t (t.tool)}
        <div class="row" style="gap:10px;flex-wrap:wrap">
          <b class="mono" style="min-width:90px">{t.tool}</b>
          <input class="mono" style="max-width:200px" aria-label={`${t.tool} version`} placeholder="version, e.g. 21.0.2-tem"
            bind:value={versions[t.tool]} autocomplete="off" />
          <button class="btn" disabled={(versions[t.tool] ?? '').trim() === (t.default ?? '')} onclick={() => saveDefault(t, versions[t.tool].trim())}>
            {versions[t.tool]?.trim() ? 'Set as default' : 'Clear default'}</button>
          {#if t.canInstall}
            <button class="btn" disabled={!versions[t.tool]?.trim() || installing !== null} onclick={() => install(t)}>Install this version</button>
          {/if}
          {#if t.default}<span class="hint">Default: <span class="mono">{t.default}</span></span>{/if}
        </div>
      {/each}
    </div>
  </section>
{/each}

{#if installing}
  <TermView path="/ws/setup/plugin-login" title={`Install ${installing}`} height="360px" onclose={() => (installing = null)}
    hint="The install runs here; close it when it ends." />
{/if}
