<script lang="ts">
  // First-run wizard: welcome → plugins → commit identity → git hosts → checks → ready.
  import Logo from '../components/Logo.svelte';
  import CheckRow from '../components/CheckRow.svelte';
  import { app, reload } from '../lib/store.svelte';
  import { get, put, post } from '../lib/api';
  import { route, go } from '../lib/router.svelte';
  import { setup, loadChecks, failingRequired } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import GitIdentity from '../components/GitIdentity.svelte';
  import GitHosts from '../components/GitHosts.svelte';
  import TermView from '../components/TermView.svelte';
  import PluginDetail from '../components/PluginDetail.svelte';
  import Panel from '../components/Panel.svelte';
  import type { ConfigView, PluginInfo } from '../lib/types';

  const STEPS = ['Welcome', 'Plugins', 'Commit identity', 'Git hosts', 'Connect & check', 'Ready'];
  const step = $derived(Math.min(Math.max(Number(route.query.get('step') ?? 1), 1), STEPS.length));
  const to = (n: number) => go('/welcome', { step: String(n) });
  const blocked = $derived(failingRequired().length > 0);
  const workspace = $derived(String(app.state?.config.workspace ?? '') || '~/xherd (default)');
  const KIND: Record<string, string> = { runtime: 'Coding agents', host: 'Git hosts', tracker: 'Issue trackers', toolchain: 'Languages' };
  const kinds = $derived([...new Set((app.state?.plugins ?? []).map((p) => p.kind))]);

  // The checks decide whether setup can be skipped, so load them on every step.
  $effect(() => { if (!setup.checks && !setup.running) loadChecks(); });

  // Steps 3 and 4 have one button: it saves what was edited, then goes on.
  let idStep = $state<GitIdentity>();
  let hostsStep = $state<GitHosts>();
  let idUnsaved = $state(false);
  let idReady = $state(false);
  let hostsUnsaved = $state(false);
  let moving = $state(false);
  const unsaved = $derived(step === 3 ? idUnsaved : step === 4 ? hostsUnsaved : false);
  async function next() {
    moving = true;
    try {
      const ok = step === 3 ? await idStep?.save() : step === 4 ? await hostsStep?.saveAll() : true;
      if (ok !== false) to(step + 1);
    } finally { moving = false; }
  }

  const plugins = $derived(app.state?.plugins ?? []);
  // The last plugin on in its kind cannot be switched off.
  const lastOn = (p: PluginInfo) => p.enabled && plugins.filter((q) => q.kind === p.kind && q.enabled).length === 1;
  const loginCheck = (p: PluginInfo) => setup.checks?.find((c) => c.id === `plugin:${p.name}:login`);
  // An enabled plugin that needs a login and is not logged in holds the step.
  const needLogin = $derived(plugins.filter((p) => p.enabled && p.login && loginCheck(p) && !loginCheck(p)!.ok));
  const toolsBlocked = $derived(needLogin.length > 0);
  let loggingIn = $state<string | null>(null);
  // The plugin whose options are open.
  let opened = $state<string | null>(null);
  // Versions found installed per tool, for the language cards (local commands, native or container).
  let installed = $state<Record<string, string[]>>({});
  let toolsByPlugin = $state<Record<string, string[]>>({});
  $effect(() => {
    if (step !== 2) return;
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
  async function startLogin(p: PluginInfo) {
    try { await post(`/plugins/${p.name}/login`); loggingIn = p.name; } catch (e) { showError(`Could not log in to ${p.label || p.name}`, e); }
  }
  function loginEnded() {
    loggingIn = null;
    loadChecks();
  }

  async function finish(next: string) {
    try {
      setup.config = await put<ConfigView>('/config', { welcomed: true });
      go(next);
    } catch (e) {
      showError('Could not save', e);
    }
  }
</script>

<div class="x">
  <header class="bar whead">
    <div class="row"><span class="logo row"><Logo /><b>XHERD</b></span><span class="grow"></span>
      {#if setup.checks && !blocked}
        <button class="bb skip" onclick={() => finish('/')}>Skip setup</button>
      {/if}</div>
    <ol class="wsteps">
      {#each STEPS as s, i (s)}
        {#if i > 0}<li class="wsep" aria-hidden="true"></li>{/if}
        <li class="wstep" class:done={i + 1 < step} class:on={i + 1 === step}
          aria-current={i + 1 === step ? 'step' : undefined}>
          <i>{i + 1 < step ? '✓' : i + 1}</i><span class="wl">{s}</span>
        </li>
      {/each}
    </ol>
  </header>

  <main class="stack narrow wmain">
    {#if step === 1}
      <h1 class="big">Run many coding agents, decide only what matters</h1>
      <p class="muted lead">xherd starts each agent in its own git worktree and terminal session, follows it through spec, design, tasks, build, review and ship, and stops it at the gates where your call is needed. Everything else runs on its own.</p>
      <p class="muted">A few steps set it up: the plugins it uses, who commits, how it reaches your git hosts, and a check that everything works.</p>
      <h2>Where xherd runs</h2>
      <Panel>
        <dl class="kv">
          <dt>Mode</dt><dd>{#if app.state?.sandbox}In the xherd <b>Docker container</b>{:else}Native on <b>{app.state?.platform}</b>{/if}, version {app.state?.buildVersion}</dd>
          <dt>Workspace</dt><dd class="mono">{workspace}</dd>
          <dt>Worktrees</dt><dd class="muted">One folder per repo, one worktree per agent, under the workspace.</dd>
        </dl>
        {#if app.state?.sandbox}
          <p class="hint">The workspace is mounted at the same path on your computer, so your editor opens worktrees directly. Languages your repositories ask for are installed in the container on first use.</p>
        {:else}
          <p class="hint">To run xherd in Docker instead, run <span class="mono">xherd docker init</span> on this computer. Change the workspace in Settings.</p>
        {/if}
      </Panel>
    {:else if step === 2}
      <h1>Plugins</h1>
      <p class="muted">Switch on what you use; at least one of each kind stays on. A card says whether the plugin is ready; open Options to set it up.</p>
      {#each kinds as k (k)}
        <span class="lbl">{KIND[k] ?? k}</span>
        <div class="cards">
          {#each plugins.filter((p) => p.kind === k) as p (p.name)}
            {@const st = pstate(p)}
            {@const lc = p.enabled && p.login ? loginCheck(p) : undefined}
            <div class="tool" class:on={p.enabled}>
              <label class="tp"><input type="checkbox" checked={p.enabled}
                  disabled={lastOn(p)} title={lastOn(p) ? 'At least one is needed' : ''} onchange={() => toggle(p)} />
                <span class="tick" aria-hidden="true">✓</span>
                <div><h3>{p.label || p.name}</h3><p class="hint">{p.description}</p></div></label>
              <div class="tf">
                <span class="ckst {st.tone === 'off' ? '' : st.tone}" aria-hidden="true">{st.tone === 'ok' ? '✓' : st.tone === 'bad' ? '✕' : st.tone === 'opt' ? '–' : '○'}</span>
                <span class="grow wrap-any" class:muted={st.tone === 'off'}>{st.text}</span>
                {#if st.text === 'Not logged in'}<button class="btn primary sm" onclick={() => startLogin(p)}>Log in</button>{/if}
                {#if p.enabled}<button class="btn sm" onclick={() => (opened = opened === p.name ? null : p.name)}>Options</button>{/if}
              </div>
            </div>
          {/each}
        </div>
        {#each plugins.filter((p) => p.kind === k && p.name === opened && p.enabled) as p (p.name)}
          <PluginDetail plugin={p} onclose={() => (opened = null)} />
        {/each}
      {/each}
      {#if loggingIn}
        <TermView path="/ws/setup/plugin-login" title={`Log in to ${plugins.find((p) => p.name === loggingIn)?.label ?? loggingIn}`}
          height="360px" onclose={loginEnded} hint="Follow the steps; the check runs again when it ends." />
      {/if}
      {#if toolsBlocked}<p class="hint">To continue, log in to {needLogin.map((p) => p.label || p.name).join(' and ')} or switch {needLogin.length > 1 ? 'them' : 'it'} off.</p>{/if}
    {:else if step === 3}
      <h1>Commit identity</h1>
      <p class="muted">Who agents commit as. Git refuses to commit without a name and email.</p>
      <GitIdentity heading={false} bar={false} bind:this={idStep} bind:unsaved={idUnsaved} bind:ready={idReady} />
    {:else if step === 4}
      <h1>Git hosts</h1>
      <p class="muted">How agents fetch and push: your SSH agent, a browser login to GitHub or GitLab, or a token. Check each host before you go on; without access agents cannot reach private repositories or push, and the same page is in Settings later.</p>
      <GitHosts heading={false} bar={false} bind:this={hostsStep} bind:unsaved={hostsUnsaved} />
    {:else if step === 5}
      <h1>Connect &amp; check</h1>
      <p class="muted">Required checks must pass before agents can start.</p>
      <Panel label="Checks" flush>
        {#if !setup.checks}<div class="cb"><p class="empty">Running checks…</p></div>{/if}
        {#each setup.checks ?? [] as c, i (c.id)}<CheckRow check={c} first={i === 0} />{/each}
      </Panel>
      <div><button class="btn" disabled={setup.running} onclick={() => loadChecks()}>Re-run</button></div>
    {:else}
      <h1>{blocked ? 'Almost ready' : 'Ready'}</h1>
      <p class="muted">{blocked ? 'Some required checks still fail; Setup checks lists them.' : 'Start your first agent on a ticket, or open the herd.'}</p>
    {/if}

    <div class="row end wnav">
      {#if step > 1}<button class="btn ghost lg" onclick={() => to(step - 1)}>Back</button>{/if}
      {#if step < STEPS.length}
        <button class="btn primary lg" disabled={moving || (step === 2 && toolsBlocked) || (step === 3 && !idReady)} onclick={next}>
          {moving ? 'Saving…' : unsaved ? 'Save and continue' : `Next: ${STEPS[step].toLowerCase()}`}</button>
      {:else if blocked}
        <button class="btn primary lg" onclick={() => finish('/setup')}>Open setup checks</button>
      {:else}
        <button class="btn lg" onclick={() => finish('/')}>Open the herd</button>
        <button class="btn primary lg" onclick={() => finish('/new')}>Start an agent</button>
      {/if}
    </div>
  </main>
</div>

<style>
  .whead { flex-direction: column; align-items: stretch; padding: 16px var(--gut) 20px; gap: 20px; }
  .whead .logo { gap: 9px; }
  .skip { width: auto; padding: 0 12px; font: 500 12.5px var(--sans); }
  .wsteps { display: flex; align-items: center; list-style: none; margin: 0; padding: 0; gap: 10px; overflow-x: auto; scrollbar-width: none; }
  .wmain { width: 100%; padding-top: 36px; }
  .big { font-size: 28px; }
  .lead { font-size: 14px; }
  .wnav { border-top: 1px solid var(--fence); padding-top: 18px; margin-top: 8px; }
  @media (max-width: 560px) { .big { font-size: 22px; } .wnav .btn { flex: 1 1 auto; } }
</style>
