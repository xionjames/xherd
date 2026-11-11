<script lang="ts">
  // The tools of one toolchain plugin (R-653), shown on its plugin page: a
  // default version per tool, used by a worktree without a version file, and
  // installing a version (container only, in a live terminal). The versions to
  // pick come from the plugin's `list` command (container only; elsewhere, or
  // if it fails, the version is typed). A default applies to agents started
  // after the daemon restarts, like the other saved settings.
  import { get, put, post } from '../lib/api';
  import { loadConfig } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';

  interface Tool { plugin: string; pluginLabel: string; tool: string; default: string | null; canInstall: boolean }
  let tools = $state<Tool[]>([]);
  let allTools = $state<Tool[]>([]);
  let sandbox = $state(false);
  let versions = $state<Record<string, string>>({});
  let installing = $state<string | null>(null);
  interface Ver { version: string; installed: boolean }
  /** Per tool: the offered versions, or why there are none (typed instead). */
  let offered = $state<Record<string, Ver[] | string>>({});
  let { plugin }: { plugin: string } = $props();

  async function load() {
    try {
      const r = await get<{ tools: Tool[]; sandbox: boolean }>('/toolchains');
      allTools = r.tools;
      tools = r.tools.filter((t) => t.plugin === plugin);
      sandbox = r.sandbox;
      versions = Object.fromEntries(tools.map((t) => [t.tool, t.default ?? '']));
      if (sandbox) for (const t of tools) loadVersions(t.tool);
    } catch (e) { showError('Could not load toolchains', e); }
  }
  async function loadVersions(tool: string) {
    offered[tool] = 'loading';
    try {
      offered[tool] = (await get<{ versions: Ver[] }>(`/toolchains/${encodeURIComponent(tool)}/versions`)).versions;
    } catch (e) {
      offered[tool] = e instanceof Error ? e.message : 'could not list versions';
    }
  }
  const list = (tool: string) => { const o = offered[tool]; return Array.isArray(o) ? o : null; };
  $effect(() => { load(); });

  async function saveDefault(t: Tool, value: string) {
    const toolchainDefaults = Object.fromEntries(allTools.map((x) => [x.tool, x.tool === t.tool ? value : (x.default ?? '')]));
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
</script>

<p class="muted" style="max-width:72ch">A worktree's own version files (<span class="mono">.nvmrc</span>, <span class="mono">.sdkmanrc</span>, <span class="mono">go.mod</span>…) always win.
  The default is what an agent gets for a tool its repository does not pin. Leave it empty for no default.</p>
{#if !sandbox}
  <p class="hint">xherd runs natively: it never installs toolchains on your computer, so only the defaults can be set here. Run xherd in Docker to install versions.</p>
{/if}

<section class="sec">
  <div class="sh"><h2>Tools</h2></div>
  <div class="bd">
    {#each tools as t (t.tool)}
      <div class="row" style="gap:10px;flex-wrap:wrap">
        <b class="mono" style="min-width:90px">{t.tool}</b>
        {#if list(t.tool)}
          <select class="mono" style="max-width:240px" aria-label={`${t.tool} version`} bind:value={versions[t.tool]}>
            <option value="">(none)</option>
            {#if versions[t.tool] && !list(t.tool)?.some((v) => v.version === versions[t.tool])}
              <option value={versions[t.tool]}>{versions[t.tool]}</option>
            {/if}
            {#each list(t.tool) ?? [] as v (v.version)}
              <option value={v.version}>{v.version}{v.installed ? ' · installed' : ''}</option>
            {/each}
          </select>
        {:else}
          <input class="mono" style="max-width:200px" aria-label={`${t.tool} version`} placeholder="version, e.g. 21.0.2-tem"
            bind:value={versions[t.tool]} autocomplete="off" />
          {#if offered[t.tool] === 'loading'}<span class="hint">Listing versions…</span>
          {:else if typeof offered[t.tool] === 'string'}<span class="hint">Could not list versions ({offered[t.tool]}); type one.</span>{/if}
        {/if}
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

{#if installing}
  <TermView path="/ws/setup/plugin-login" title={`Install ${installing}`} height="360px" onclose={() => (installing = null)}
    hint="The install runs here; close it when it ends." />
{/if}
