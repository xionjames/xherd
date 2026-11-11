<script lang="ts">
  // Board (spec §17): the tasks of every agent in columns by status. Read-only;
  // the filter (`?agent=NAME`) lives in the URL. Stage cards and TODOs are not here yet.
  import { app } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import type { Task } from '../lib/types';

  const COLUMNS: [Task['status'], string][] = [['todo', 'To do'], ['doing', 'Doing'], ['blocked', 'Blocked'], ['done', 'Done']];
  const only = $derived(route.query.get('agent') ?? '');
  const agents = $derived((app.state?.agents ?? []).filter((a) => a.tasks?.length));
  const cards = $derived(
    agents.filter((a) => !only || a.name === only).flatMap((a) => (a.tasks ?? []).map((t) => ({ agent: a.name, task: t }))),
  );
</script>

<div class="stack">
  <div class="phead notabs"><div class="row"><h1>Board</h1>
    <span class="grow"></span>
    <select aria-label="Agent" value={only} onchange={(e) => (location.hash = href('/board', { agent: e.currentTarget.value }))}>
      <option value="">All agents</option>
      {#each agents as a (a.id)}<option value={a.name}>{a.name}</option>{/each}
    </select></div></div>
  {#if !agents.length}
    <p class="muted">No agent has a task list yet. Tasks appear here once an agent's tasks are approved.</p>
  {:else}
    <div class="cols">
      {#each COLUMNS as [st, label] (st)}
        {@const list = cards.filter((c) => c.task.status === st)}
        <section class="col" aria-label={label}>
          <h2>{label} <span class="hint num">{list.length}</span></h2>
          {#each list as c (`${c.agent}/${c.task.id}`)}
            <a class="card tk" href={href(`/agent/${c.agent}`, { s: 'tasks' })}>
              <span class="hint mono">{c.agent} · {c.task.id}</span>
              <span>{c.task.title}</span>
              {#if c.task.blockedReason}<span class="hint">Blocked: {c.task.blockedReason}</span>{/if}
            </a>
          {/each}
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; align-items: start; }
  .col { display: flex; flex-direction: column; gap: 8px; }
  .col h2 { font-size: 13px; margin: 0; }
  .tk { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; color: inherit; text-decoration: none; font-size: 13px; }
  .tk:hover { background: var(--accent-wash); }
</style>
