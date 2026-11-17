<script lang="ts">
  import GlobalBar from './components/GlobalBar.svelte';
  import ErrorDialog from './components/ErrorDialog.svelte';
  import Herd from './pages/Herd.svelte';
  import Job from './pages/Job.svelte';
  import Settings from './pages/Settings.svelte';
  import Setup from './pages/Setup.svelte';
  import Welcome from './pages/Welcome.svelte';
  import NewJob from './pages/NewJob.svelte';
  import Inbox from './pages/Inbox.svelte';
  import SkillsPage from './pages/SkillsPage.svelte';
  import Board from './pages/Board.svelte';
  import Time from './pages/Time.svelte';
  import Todos from './pages/Todos.svelte';
  import Agents from './pages/Agents.svelte';
  import AgentDef from './pages/AgentDef.svelte';
  import Flows from './pages/Flows.svelte';
  import Callout from './components/Callout.svelte';
  import { app, start, agentByName } from './lib/store.svelte';
  import { route, go } from './lib/router.svelte';
  import { setup, loadChecks, loadConfig, failingRequired } from './lib/setup.svelte';

  start();
  loadChecks().catch((e) => console.error('checks', e));
  loadConfig().catch((e) => console.error('config', e));

  const jobId = $derived(route.path.match(/^\/jobs\/([^/]+)/)?.[1]);
  const job = $derived(jobId ? app.state?.jobs.find((a) => a.name === jobId || a.id === jobId) : undefined);
  // `#/agents/<name>` is a saved agent (issue #141); `new` is the empty form.
  const agentName = $derived(route.path.match(/^\/agents\/([^/]+)$/)?.[1]);
  // Old links: `#/agent/<name>` opens the job, and so does `#/agents/<name>`
  // when no saved agent has that name.
  $effect(() => {
    const old = route.path.match(/^\/agent\/([^/]+)$/)?.[1]
      ?? (agentName && agentName !== 'new' && app.state && !agentByName(agentName) && app.state.jobs.some((j) => j.name === agentName) ? agentName : undefined);
    if (old) location.replace(`#/jobs/${old}${route.query.size ? `?${route.query}` : ''}`);
  });
  // Old names: Needs attention is the Inbox, the Skills tab of Settings is Team → Skills.
  $effect(() => {
    if (route.path === '/attention' || route.path === '/review') location.replace('#/inbox');
    else if (route.path === '/settings' && route.query.get('tab') === 'skills') location.replace('#/skills');
  });
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
    {#if !app.state}
      <div class="stack"><p class="empty">{app.error ? `Cannot reach the daemon: ${app.error}` : 'Loading…'}</p></div>
    {:else if route.path === '/setup'}
      <Setup />
    {:else if route.path === '/settings'}
      <Settings />
    {:else if route.path === '/todos'}
      <Todos />
    {:else if route.path === '/time'}
      <Time />
    {:else if route.path === '/board'}
      <Board />
    {:else if route.path === '/inbox'}
      <Inbox />
    {:else if route.path === '/skills'}
      <SkillsPage />
    {:else if route.path === '/agents'}
      <Agents />
    {:else if route.path === '/flows' || route.path.startsWith('/flows/')}
      <Flows />
    {:else if agentName}
      {#key agentName}<AgentDef name={agentName} />{/key}
    {:else if route.path === '/new'}
      <NewJob />
    {:else if route.path === '/'}
      <Herd />
    {:else if jobId && job}
      <Job agent={job} />
    {:else if jobId}
      <div class="stack"><p class="empty">No job {jobId}. <a href="#/">Back to the jobs</a>.</p></div>
    {:else}
      <div class="stack"><p class="empty">Nothing here. <a href="#/">Back to the jobs</a>.</p></div>
    {/if}
    {#if app.state && !app.connected}
      <Callout tone="bad" class="toast">Reconnecting to the daemon…</Callout>
    {/if}
  </div>
{/if}
<ErrorDialog />
