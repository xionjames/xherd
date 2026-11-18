<script lang="ts">
  // The plugin view of Welcome step 2 and Settings → Plugins (R1): search, broken
  // plugins, then an Enabled / Available switch and a kind filter, cards grouped by kind. A card switches its
  // plugin on or off; Options (and Log in) open PluginDetail in a dialog.
  import { onMount } from 'svelte';
  import { app, reload } from '../lib/store.svelte';
  import { get, put, pluginsList } from '../lib/api';
  import { setup, loadChecks, loadConfig } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import { KIND_LABEL, KIND_ONE, KIND_ORDER, lastOn, matches } from '../lib/plugins';
  import Seg from './Seg.svelte';
  import PluginDetail from './PluginDetail.svelte';
  import PluginAdd from './PluginAdd.svelte';
  import type { PluginInfo, PluginsList } from '../lib/types';

  let { selected, onselect }: { selected: string | null; onselect: (name: string | null) => void } = $props();

  const plugins = $derived(app.state?.plugins ?? []);
  const broken = $derived(app.state?.brokenPlugins ?? []);
  let query = $state('');
  const shown = $derived(plugins.filter((p) => matches(p, query)));
  const kinds = $derived([...KIND_ORDER, ...new Set(plugins.map((p) => p.kind).filter((k) => !KIND_ORDER.includes(k)))]);
  // Enabled and Available (installed but switched off) are two views; the kind filter narrows both.
  let view = $state<'enabled' | 'available'>('enabled');
  let kind = $state('all');
  const byKind = $derived(shown.filter((p) => kind === 'all' || p.kind === kind));
  const enabledList = $derived(byKind.filter((p) => p.enabled));
  const availableList = $derived(byKind.filter((p) => !p.enabled));
  const viewList = $derived(view === 'enabled' ? enabledList : availableList);
  const otherCount = $derived((view === 'enabled' ? availableList : enabledList).length);
  const kindOptions = $derived([
    { value: 'all', label: 'All' },
    ...kinds.filter((k) => plugins.some((p) => p.kind === k)).map((k) => ({ value: k, label: KIND_LABEL[k] ?? k })),
  ]);
  const open = $derived(plugins.find((p) => p.name === selected));
  // The plugin whose dialog was opened by its card's Log in: the dialog starts the login.
  let autologin = $state<string | null>(null);

  // Versions found installed per tool, for the language cards (local commands, native or container).
  let installed = $state<Record<string, string[]>>({});
  let toolsByPlugin = $state<Record<string, string[]>>({});
  // Installing from a git URL: off with XHERD_PLUGIN_INSTALL=off; a change needs a restart.
  let list = $state<PluginsList | null>(null);
  let adding = $state(false);
  const refreshList = () => pluginsList().then((r) => (list = r)).catch(() => {});
  async function afterInstall() {
    await refreshList();
    await reload();
    await loadConfig().catch(() => {});
  }
  const short = (s: { url: string; sha?: string | null }) => `${s.url}${s.sha ? `@${s.sha.slice(0, 7)}` : ''}`;
  onMount(() => {
    refreshList();
    get<{ installed: Record<string, string[]> }>('/toolchains/installed').then((r) => (installed = r.installed)).catch(() => {});
    get<{ tools: { plugin: string; tool: string }[] }>('/toolchains').then((r) => {
      const m: Record<string, string[]> = {};
      for (const t of r.tools) (m[t.plugin] ??= []).push(t.tool);
      toolsByPlugin = m;
    }).catch(() => {});
  });
  // "node v20.11.0, v18.19.1 installed", at most three versions.
  function foundText(p: PluginInfo): string {
    const parts = (toolsByPlugin[p.name] ?? []).flatMap((t) => {
      const v = installed[t] ?? [];
      return v.length ? [`${t} ${v.slice(0, 3).join(', ')}${v.length > 3 ? ` +${v.length - 3}` : ''}`] : [];
    });
    return parts.join('; ');
  }
  // What a card says without being opened: off, ready, or not ready and why.
  function pstate(p: PluginInfo): { tone: 'off' | 'ok' | 'bad' | 'opt'; text: string } {
    if (!p.enabled) return { tone: 'off', text: 'Off' };
    const bad = (setup.checks ?? []).find((c) => c.id.startsWith(`plugin:${p.name}:`) && !c.ok);
    if (bad) return { tone: bad.required ? 'bad' : 'opt', text: bad.id.endsWith(':login') ? 'Not logged in' : bad.detail.split('\n')[0] };
    const saved = ((setup.config?.config.pluginSettings ?? {}) as Record<string, Record<string, unknown>>)[p.name] ?? {};
    const missing = p.settings.find((s) => s.required && !s.secret && !saved[s.key]);
    if (missing) return { tone: 'bad', text: `Needs ${missing.label || missing.key}` };
    // A language manager with no versions yet is the normal state: they install on first use.
    if (p.kind === 'toolchain') {
      const found = foundText(p);
      if (found) return { tone: 'ok', text: `Ready · ${found} installed` };
      if (app.state?.sandbox) return { tone: 'ok', text: 'Ready · versions install on first use' };
    }
    return { tone: 'ok', text: 'Ready' };
  }

  async function toggle(p: PluginInfo) {
    const off = plugins.filter((q) => (q.name === p.name ? q.enabled : !q.enabled)).map((q) => q.name);
    try {
      await put('/config', { disabledPlugins: off });
      await reload();
      await loadChecks();
    } catch (e) {
      showError('Could not change the plugin', e);
    }
  }
  function options(p: PluginInfo, login = false) {
    autologin = login ? p.name : null;
    onselect(p.name);
  }
  function close() {
    autologin = null;
    onselect(null);
  }
</script>

<div class="row">
  <Seg label="Plugins" value={view} onchange={(v) => (view = v)} options={[
    { value: 'enabled', label: `Enabled ${plugins.filter((p) => p.enabled).length}` },
    { value: 'available', label: 'Available', title: 'Installed but switched off' },
  ]} />
  <input class="grow" type="search" placeholder="Search plugins" aria-label="Filter plugins" bind:value={query} />
  {#if list?.installEnabled !== false}<button class="btn" onclick={() => (adding = true)}>Add plugin</button>{/if}
</div>
{#if list?.restartNeeded}
  <p class="hint"><b>Restart needed</b> for the plugin changes to load. Running agents keep going.</p>
{/if}
{#if list?.installEnabled === false}
  <p class="hint">Installing plugins is switched off here (<span class="mono">XHERD_PLUGIN_INSTALL=off</span>).</p>
{/if}

{#if broken.length}
  <div class="cards">
    {#each broken as b (b.dir)}
      <article class="pcard err">
        <div class="h"><span class="mi bad" aria-hidden="true">!</span>
          <div class="wrap-any"><h3>{b.name}</h3><div class="hint mono">{b.dir}</div></div></div>
        <div class="b"><p class="err">{b.error}</p></div>
      </article>
    {/each}
  </div>
{/if}

{#if kindOptions.length > 2}
  <Seg label="Kind" value={kind} onchange={(v) => (kind = v)} options={kindOptions} />
{/if}
{#if view === 'available'}<p class="hint">Installed but switched off.</p>{/if}

{#each kinds.filter((k) => viewList.some((p) => p.kind === k)) as k (k)}
  {#if kind === 'all'}<span class="lbl">{KIND_LABEL[k] ?? k}</span>{/if}
  <div class="cards">
    {#each viewList.filter((p) => p.kind === k) as p (p.name)}
        {@const st = pstate(p)}
        {@const last = lastOn(p, plugins)}
        <div class="tool" class:on={p.enabled}>
          <label class="tp"><input type="checkbox" checked={p.enabled}
              disabled={last} title={last ? 'At least one is needed' : ''} onchange={() => toggle(p)} />
            <span class="tick" aria-hidden="true">✓</span>
            <div><h3>{p.label || p.name}</h3><p class="hint">{p.description}</p>
              <p class="hint">{KIND_ONE[p.kind] ?? p.kind} · {p.version}{p.builtin ? ' · built in' : ''}</p>
              {#if p.source}<p class="hint wrap-any">from {short(p.source)}</p>{:else if !p.builtin}<p class="hint">installed by hand, not pinned</p>{/if}</div></label>
          <div class="tf">
            <span class="ckst {st.tone === 'off' ? '' : st.tone}" aria-hidden="true">{st.tone === 'ok' ? '✓' : st.tone === 'bad' ? '✕' : st.tone === 'opt' ? '–' : '○'}</span>
            <span class="grow wrap-any" class:muted={st.tone === 'off'}>{st.text}</span>
            {#if st.text === 'Not logged in' && p.login}<button class="btn sm" onclick={() => options(p, true)}>Log in</button>{/if}
            <button class="btn sm" onclick={() => options(p)}>Options</button>
          </div>
        </div>
    {/each}
  </div>
{:else}
  <p class="empty">{query.trim() || kind !== 'all'
    ? `No ${view} plugin matches.${otherCount ? ` ${otherCount} in the other tab.` : ''}`
    : view === 'enabled' ? 'No plugin is enabled.' : 'Every installed plugin is enabled.'}</p>
{/each}

{#if adding}
  <PluginAdd onclose={() => (adding = false)} oninstalled={afterInstall} />
{/if}

{#if open}
  {#key open.name}<PluginDetail plugin={open} autologin={autologin === open.name} onclose={close} />{/key}
{/if}
