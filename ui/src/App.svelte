<script lang="ts">
  import GlobalBar from './components/GlobalBar.svelte';
  import HerdStrip from './components/HerdStrip.svelte';
  import Herd from './pages/Herd.svelte';
  import Agent from './pages/Agent.svelte';
  import ErrorDialog from './components/ErrorDialog.svelte';
  import { app, start } from './lib/store.svelte';
  import { route } from './lib/router.svelte';

  start();

  const agentId = $derived(route.path.match(/^\/agent\/([^/]+)/)?.[1]);
  const agent = $derived(agentId ? app.state?.agents.find((a) => a.name === agentId || a.id === agentId) : undefined);
  const current = $derived(agent?.id);
</script>

<div class="x">
  <GlobalBar />
  <HerdStrip {current} />
  {#if !app.state}
    <div class="stack"><p class="muted">{app.error ? `Cannot reach the daemon: ${app.error}` : 'Loading…'}</p></div>
  {:else if route.path === '/'}
    <Herd />
  {:else if agentId && agent}
    <Agent {agent} />
  {:else if agentId}
    <div class="stack"><p class="muted">No agent {agentId}. <a href="#/">Back to the herd</a>.</p></div>
  {:else}
    <div class="stack"><p class="muted">Nothing here. <a href="#/">Back to the herd</a>.</p></div>
  {/if}
  <ErrorDialog />
  {#if app.state && !app.connected}
    <div class="offline" role="status">Reconnecting to the daemon…</div>
  {/if}
</div>

<style>
  .offline { position: fixed; bottom: 16px; left: 50%; transform: translateX(-50%); padding: 8px 14px; border-radius: 8px;
    background: var(--halt-wash); color: var(--halt); border: 1px solid var(--halt); font-weight: 500; }
</style>
