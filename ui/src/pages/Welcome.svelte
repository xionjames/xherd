<script lang="ts">
  // First-run wizard: welcome → where it runs → tools → checks → ready.
  import Logo from '../components/Logo.svelte';
  import CheckRow from '../components/CheckRow.svelte';
  import { app, reload } from '../lib/store.svelte';
  import { put, post } from '../lib/api';
  import { route, go } from '../lib/router.svelte';
  import { setup, loadChecks, failingRequired } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import GitAccess from '../components/GitAccess.svelte';
  import TermView from '../components/TermView.svelte';
  import Toolchains from '../components/Toolchains.svelte';
  import Panel from '../components/Panel.svelte';
  import type { ConfigView, PluginInfo } from '../lib/types';

  const STEPS = ['Welcome', 'Where it runs', 'Your tools', 'Git access', 'Connect & check', 'Ready'];
  const step = $derived(Math.min(Math.max(Number(route.query.get('step') ?? 1), 1), STEPS.length));
  const to = (n: number) => go('/welcome', { step: String(n) });
  const blocked = $derived(failingRequired().length > 0);
  const workspace = $derived(String(app.state?.config.workspace ?? '') || '~/xherd (default)');
  const KIND: Record<string, string> = { runtime: 'Coding agents', host: 'Git hosts', tracker: 'Issue trackers', toolchain: 'Languages' };
  const kinds = $derived([...new Set((app.state?.plugins ?? []).map((p) => p.kind))]);

  // The checks decide whether setup can be skipped, so load them on every step.
  $effect(() => { if (!setup.checks && !setup.running) loadChecks(); });

  const plugins = $derived(app.state?.plugins ?? []);
  // The last plugin on in its kind cannot be switched off.
  const lastOn = (p: PluginInfo) => p.enabled && plugins.filter((q) => q.kind === p.kind && q.enabled).length === 1;
  const loginCheck = (p: PluginInfo) => setup.checks?.find((c) => c.id === `plugin:${p.name}:login`);
  // An enabled plugin that needs a login and is not logged in holds the step.
  const toolsBlocked = $derived(plugins.some((p) => p.enabled && p.login && loginCheck(p) && !loginCheck(p)!.ok));
  let loggingIn = $state<string | null>(null);
  // Toolchain plugin whose versions are open (they are listed only when opened).
  let versionsOpen = $state<string | null>(null);

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
      <p class="muted">A few steps set it up: where it runs, which tools it uses, and a check that everything works.</p>
    {:else if step === 2}
      <h1>Where xherd runs</h1>
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
    {:else if step === 3}
      <h1>Your tools</h1>
      <p class="muted">Switch on what you use; at least one of each kind stays on. A plugin that needs a login is set up here.</p>
      {#each kinds as k (k)}
        <span class="lbl">{KIND[k] ?? k}</span>
        <div class="cards">
          {#each plugins.filter((p) => p.kind === k) as p (p.name)}
            {@const lc = p.enabled && p.login ? loginCheck(p) : undefined}
            <div class="tool" class:on={p.enabled}>
              <label class="tp"><input type="checkbox" checked={p.enabled}
                  disabled={lastOn(p)} title={lastOn(p) ? 'At least one is needed' : ''} onchange={() => toggle(p)} />
                <span class="tick" aria-hidden="true">✓</span>
                <div><h3>{p.label || p.name}</h3><p class="hint">{p.description}</p></div></label>
              <div class="tf">
                <span class="state">{p.enabled ? 'On' : 'Off'}</span>
                <span class="grow">· {p.builtin ? 'Built in' : 'Installed'}</span>
                {#if lc}
                  <span class="ckst {lc.ok ? 'ok' : 'bad'}" aria-label={lc.ok ? 'logged in' : 'not logged in'}>{lc.ok ? '✓' : '✕'}</span>
                  {lc.ok ? 'Logged in' : 'Not logged in'}
                  {#if !lc.ok}<button class="btn primary sm" onclick={() => startLogin(p)}>Log in</button>{/if}
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {/each}
      {#if app.state?.sandbox}
        {@const langs = plugins.filter((p) => p.kind === 'toolchain' && p.enabled)}
        {#if langs.length}
          <span class="lbl">Language versions</span>
          <p class="hint">Optional: install a version now, or set the default for repositories that do not pin one. Otherwise versions install on first use. You can change all of this later in Settings → Plugins.</p>
          {#each langs as p (p.name)}
            <details ontoggle={(e) => { if ((e.currentTarget as HTMLDetailsElement).open) versionsOpen = p.name; else if (versionsOpen === p.name) versionsOpen = null; }}>
              <summary>{p.label || p.name}</summary>
              {#if versionsOpen === p.name}{#key p.name}<Toolchains plugin={p.name} />{/key}{/if}
            </details>
          {/each}
        {/if}
      {/if}
      {#if loggingIn}
        <TermView path="/ws/setup/plugin-login" title={`Log in to ${plugins.find((p) => p.name === loggingIn)?.label ?? loggingIn}`}
          height="360px" onclose={loginEnded} hint="Follow the steps; the check runs again when it ends." />
      {/if}
      {#if toolsBlocked}<p class="hint">Log in to continue; Settings has the same login later.</p>{/if}
    {:else if step === 4}
      <h1>Git access</h1>
      <p class="muted">How agents fetch and push: your SSH agent, a browser login to GitHub or GitLab, or a token. Check each host before you go on; you can skip this and come back in Settings.</p>
      <GitAccess />
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
        <button class="btn primary lg" disabled={step === 3 && toolsBlocked} onclick={() => to(step + 1)}>Next: {STEPS[step].toLowerCase()}</button>
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
