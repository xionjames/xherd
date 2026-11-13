<script lang="ts">
  // First-run wizard: welcome → plugins → commit identity → git hosts → checks → ready.
  import Logo from '../components/Logo.svelte';
  import CheckRow from '../components/CheckRow.svelte';
  import { app } from '../lib/store.svelte';
  import { put } from '../lib/api';
  import { needLogin } from '../lib/plugins';
  import { route, go } from '../lib/router.svelte';
  import { setup, loadChecks, failingRequired } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import GitIdentity from '../components/GitIdentity.svelte';
  import GitHosts from '../components/GitHosts.svelte';
  import PluginList from '../components/PluginList.svelte';
  import Panel from '../components/Panel.svelte';
  import type { ConfigView } from '../lib/types';

  const STEPS = ['Welcome', 'Plugins', 'Commit identity', 'Git hosts', 'Connect & check', 'Ready'];
  const step = $derived(Math.min(Math.max(Number(route.query.get('step') ?? 1), 1), STEPS.length));
  const to = (n: number) => go('/welcome', { step: String(n) });
  const blocked = $derived(failingRequired().length > 0);
  const workspace = $derived(String(app.state?.config.workspace ?? '') || '~/xherd (default)');

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
  // An enabled plugin that needs a login and is not logged in holds the step (R3b).
  const waiting = $derived(needLogin(plugins, setup.checks));
  const toolsBlocked = $derived(waiting.length > 0);
  // The plugin whose options dialog is open.
  let opened = $state<string | null>(null);

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
      <p class="muted">Switch on what you use. Coding agents and languages keep at least one on; Git providers and issue trackers are optional. A card says whether the plugin is ready; open Options to set it up.</p>
      <PluginList selected={opened} onselect={(n) => (opened = n)} />
      {#if toolsBlocked}<p class="hint">To continue, log in to {waiting.map((p) => p.label || p.name).join(' and ')} or switch {waiting.length > 1 ? 'them' : 'it'} off.</p>{/if}
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
