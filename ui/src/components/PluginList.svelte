<script lang="ts">
  // The plugin view of Welcome step 2 and Settings → Plugins (R1): search, broken
  // plugins, then Enabled and Available grouped by kind. A card switches its
  // plugin on or off; Options (and Log in) open PluginDetail in a dialog.
  import { onMount } from 'svelte';
  import { app, reload } from '../lib/store.svelte';
  import { get, put } from '../lib/api';
  import { setup, loadChecks } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import { KIND_LABEL, KIND_ONE, KIND_ORDER, lastOn, matches } from '../lib/plugins';
  import PluginDetail from './PluginDetail.svelte';
  import type { PluginInfo } from '../lib/types';

  let { selected, onselect }: { selected: string | null; onselect: (name: string | null) => void } = $props();

  const plugins = $derived(app.state?.plugins ?? []);
  const broken = $derived(app.state?.brokenPlugins ?? []);
  let query = $state('');
  const shown = $derived(plugins.filter((p) => matches(p, query)));
  const kinds = $derived([...KIND_ORDER, ...new Set(plugins.map((p) => p.kind).filter((k) => !KIND_ORDER.includes(k)))]);
  const sections = $derived([
    { title: 'Enabled', hint: '', list: shown.filter((p) => p.enabled) },
    { title: 'Available', hint: 'Installed but switched off.', list: shown.filter((p) => !p.enabled) },
  ].filter((s) => s.list.length));
  const open = $derived(plugins.find((p) => p.name === selected));
  // The plugin whose dialog was opened by its card's Log in: the dialog starts the login.
  let autologin = $state<string | null>(null);

  // Versions found installed per tool, for the language cards (local commands, native or container).
  let installed = $state<Record<string, string[]>>({});
  let toolsByPlugin = $state<Record<string, string[]>>({});
  onMount(() => {
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

<input type="search" placeholder="Search plugins" aria-label="Filter plugins" bind:value={query} />

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

{#each sections as s (s.title)}
  <div class="sect"><h2>{s.title}</h2>{#if s.hint}<span class="hint">{s.hint}</span>{/if}</div>
  {#each kinds.filter((k) => s.list.some((p) => p.kind === k)) as k (k)}
    <span class="lbl">{KIND_LABEL[k] ?? k}</span>
    <div class="cards">
      {#each s.list.filter((p) => p.kind === k) as p (p.name)}
        {@const st = pstate(p)}
        {@const last = lastOn(p, plugins)}
        <div class="tool" class:on={p.enabled}>
          <label class="tp"><input type="checkbox" checked={p.enabled}
              disabled={last} title={last ? 'At least one is needed' : ''} onchange={() => toggle(p)} />
            <span class="tick" aria-hidden="true">✓</span>
            <div><h3>{p.label || p.name}</h3><p class="hint">{p.description}</p>
              <p class="hint">{KIND_ONE[p.kind] ?? p.kind} · {p.version}{p.builtin ? ' · built in' : ''}</p></div></label>
          <div class="tf">
            <span class="ckst {st.tone === 'off' ? '' : st.tone}" aria-hidden="true">{st.tone === 'ok' ? '✓' : st.tone === 'bad' ? '✕' : st.tone === 'opt' ? '–' : '○'}</span>
            <span class="grow wrap-any" class:muted={st.tone === 'off'}>{st.text}</span>
            {#if st.text === 'Not logged in' && p.login}<button class="btn sm" onclick={() => options(p, true)}>Log in</button>{/if}
            <button class="btn sm" onclick={() => options(p)}>Options</button>
          </div>
        </div>
      {/each}
    </div>
  {/each}
{:else}
  <p class="empty">{query.trim() ? 'No plugin matches.' : 'No plugin is installed.'}</p>
{/each}

{#if open}
  {#key open.name}<PluginDetail plugin={open} autologin={autologin === open.name} onclose={close} />{/key}
{/if}
