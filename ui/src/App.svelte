<script lang="ts">
  import GlobalBar from './components/GlobalBar.svelte';
  import HerdStrip from './components/HerdStrip.svelte';
  import ErrorDialog from './components/ErrorDialog.svelte';
  import Herd from './pages/Herd.svelte';
  import Agent from './pages/Agent.svelte';
  import Settings from './pages/Settings.svelte';
  import Setup from './pages/Setup.svelte';
  import Welcome from './pages/Welcome.svelte';
  import NewAgent from './pages/NewAgent.svelte';
  import Review from './pages/Review.svelte';
  import Board from './pages/Board.svelte';
  import Time from './pages/Time.svelte';
  import { app, start } from './lib/store.svelte';
  import { route, go } from './lib/router.svelte';
  import { setup, loadChecks, loadConfig, failingRequired } from './lib/setup.svelte';

  start();
  loadChecks().catch((e) => console.error('checks', e));
  loadConfig().catch((e) => console.error('config', e));

  const agentId = $derived(route.path.match(/^\/agent\/([^/]+)/)?.[1]);
  const agent = $derived(agentId ? app.state?.agents.find((a) => a.name === agentId || a.id === agentId) : undefined);
  // Until required checks pass, only Setup and Settings are open.
  const blocked = $derived(failingRequired().length > 0);
  const firstRun = $derived(setup.config?.config.welcomed === false);

  $effect(() => {
    if (firstRun && route.path !== '/welcome') go('/welcome');
    else if (!firstRun && blocked && !['/setup', '/settings', '/welcome'].includes(route.path)) go('/setup');
  });
</script>

{#if route.path === '/welcome'}
  <Welcome />
{:else}
  <div class="x">
    <GlobalBar {blocked} />
    {#if !blocked}<HerdStrip current={agent?.id} />{/if}
    {#if !app.state}
      <div class="stack"><p class="muted">{app.error ? `Cannot reach the daemon: ${app.error}` : 'Loading…'}</p></div>
    {:else if route.path === '/setup'}
      <Setup />
    {:else if route.path === '/settings'}
      <Settings />
    {:else if route.path === '/time'}
      <Time />
    {:else if route.path === '/board'}
      <Board />
    {:else if route.path === '/review'}
      <Review />
    {:else if route.path === '/new'}
      <NewAgent />
    {:else if route.path === '/'}
      <Herd />
    {:else if agentId && agent}
      <Agent {agent} />
    {:else if agentId}
      <div class="stack"><p class="muted">No agent {agentId}. <a href="#/">Back to the herd</a>.</p></div>
    {:else}
      <div class="stack"><p class="muted">Nothing here. <a href="#/">Back to the herd</a>.</p></div>
    {/if}
    {#if app.state && !app.connected}
      <div class="offline" role="status">Reconnecting to the daemon…</div>
    {/if}
  </div>
{/if}
<ErrorDialog />

<style>
  .offline { position: fixed; bottom: 16px; left: 50%; transform: translateX(-50%); padding: 8px 14px; border-radius: 8px;
    background: var(--halt-wash); color: var(--halt); border: 1px solid var(--halt); font-weight: 500; }
</style>
