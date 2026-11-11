<script lang="ts">
  // Board (spec §17, R-210): stage cards (one per agent, in the column of its
  // flow stage's role), the tasks of every agent by status, and your TODOs.
  // Read-only. View, filter and swimlanes live in the URL:
  // `?view=stages|tasks&agent=NAME&lanes=none|repo|agent`.
  import StatusPill from '../components/StatusPill.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Seg from '../components/Seg.svelte';
  import { app } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { get } from '../lib/api';
  import { flowOf, STAGE_LABEL } from '../lib/flow';
  import { agoText, repoName } from '../lib/format';
  import type { Agent, Task } from '../lib/types';

  type Todo = { id: string; title: string; due: number; status: 'open' | 'checking' | 'due' | 'done' | 'error'; condition?: string };
  type Card =
    | { kind: 'agent'; key: string; lane: string; col: string; agent: Agent }
    | { kind: 'task'; key: string; lane: string; col: string; agent: Agent; task: Task }
    | { kind: 'todo'; key: string; lane: string; col: string; todo: Todo };

  const view = $derived(route.query.get('view') === 'tasks' ? 'tasks' : 'stages');
  const lanes = $derived((['repo', 'agent'] as const).find((l) => l === route.query.get('lanes')) ?? 'none');
  const only = $derived(route.query.get('agent') ?? '');
  const go = (q: Record<string, string | undefined>) => {
    location.hash = href('/board', { view: route.query.get('view') ?? undefined, agent: route.query.get('agent') ?? undefined, lanes: route.query.get('lanes') ?? undefined, ...q });
  };

  let todos = $state<Todo[]>([]);
  $effect(() => {
    const load = () => get<{ todos: Todo[] }>('/todos').then((r) => (todos = r.todos.filter((t) => t.status !== 'done'))).catch(() => {});
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  });

  const ROLE_ORDER = ['explore', 'spec', 'design', 'tasks', 'implement', 'review', 'ship', 'pr'];
  const TASK_COLS: [string, string][] = [['todo', 'To do'], ['doing', 'Doing'], ['blocked', 'Blocked'], ['done', 'Done']];
  const NOT_STARTED = 'start';
  const FINISHED = 'finished';
  const TODOS = 'todos';

  const everyone = $derived(app.state?.agents ?? []);
  const shown = $derived(everyone.filter((a) => !only || a.name === only));
  const laneOf = (a: Agent) => (lanes === 'repo' ? repoName(a) : lanes === 'agent' ? a.name : '');

  /** The column of an agent's current stage: its role (R-210), so a setting or
   *  card keyed by role looks the same in every pack. */
  function stageCol(a: Agent): string {
    if (a.status === 'done' || a.status === 'watching') return FINISHED;
    const stage = a.stage ? flowOf(a, app.state?.flows ?? [])?.stages.find((s) => s.id === a.stage) : undefined;
    return stage ? stage.role : NOT_STARTED;
  }

  const cards = $derived.by((): Card[] => {
    const out: Card[] = [];
    if (view === 'stages') {
      for (const a of shown) out.push({ kind: 'agent', key: a.id, lane: laneOf(a), col: stageCol(a), agent: a });
    } else {
      for (const a of shown) for (const t of a.tasks ?? []) out.push({ kind: 'task', key: `${a.id}/${t.id}`, lane: laneOf(a), col: t.status, agent: a, task: t });
    }
    if (!only) for (const t of todos) out.push({ kind: 'todo', key: `todo:${t.id}`, lane: lanes === 'none' ? '' : 'TODOs', col: TODOS, todo: t });
    return out;
  });

  const columns = $derived.by((): [string, string][] => {
    const base: [string, string][] = view === 'tasks' ? TASK_COLS : stageColumns();
    return cards.some((c) => c.kind === 'todo') ? [...base, [TODOS, 'TODOs']] : base;
  });
  function stageColumns(): [string, string][] {
    const used = new Set(cards.filter((c) => c.kind === 'agent').map((c) => c.col));
    const roles = new Set((app.state?.flows ?? []).flatMap((f) => f.stages.map((s) => s.role as string)));
    const cols: [string, string][] = [];
    if (used.has(NOT_STARTED)) cols.push([NOT_STARTED, 'Not started']);
    for (const r of ROLE_ORDER) if (roles.has(r) || used.has(r)) cols.push([r, STAGE_LABEL[r] ?? r]);
    for (const r of used) if (!cols.some(([c]) => c === r) && r !== FINISHED) cols.push([r, r]);
    cols.push([FINISHED, 'Finished']);
    return cols;
  }

  const laneNames = $derived([...new Set(cards.map((c) => c.lane))].sort((a, b) => (a === 'TODOs' ? 1 : b === 'TODOs' ? -1 : a.localeCompare(b))));
  const agentLinks = $derived(everyone.filter((a) => view === 'stages' || a.tasks?.length));
  const empty = $derived(!cards.length);
</script>

<PageHead title="Board">
  {#snippet actions()}
    <Seg label="View" value={view} onchange={(v) => go({ view: v === 'tasks' ? 'tasks' : undefined })}
      options={[{ value: 'stages', label: 'Stages' }, { value: 'tasks', label: 'Tasks' }]} />
    <select aria-label="Swimlanes" value={lanes} onchange={(e) => go({ lanes: e.currentTarget.value === 'none' ? undefined : e.currentTarget.value })}>
      <option value="none">No swimlanes</option><option value="repo">Lanes by repository</option><option value="agent">Lanes by agent</option>
    </select>
    <select aria-label="Agent" value={only} onchange={(e) => go({ agent: e.currentTarget.value || undefined })}>
      <option value="">All agents</option>
      {#each agentLinks as a (a.id)}<option value={a.name}>{a.name}</option>{/each}
    </select>
  {/snippet}
</PageHead>

<div class="stack">
  {#if empty}
    <p class="empty">{view === 'tasks' ? "No agent has a task list yet. Tasks appear here once an agent's tasks are approved." : 'No agents yet.'}</p>
  {:else}
    {#each laneNames as lane (lane)}
      <section class="lane" aria-label={lane || 'Board'}>
        {#if lane}<h2 class="lh">{lane}</h2>{/if}
        <div class="cols">
          {#each columns as [col, label] (col)}
            {@const list = cards.filter((c) => c.lane === lane && c.col === col)}
            <section class="col" aria-label={label}>
              <h3>{label} <span class="hint num">{list.length}</span></h3>
              {#each list as c (c.key)}
                {#if c.kind === 'agent'}
                  <a class="card tk" href={href(`/agent/${c.agent.name}`)}>
                    <span class="mono"><b>{c.agent.name}</b></span>
                    <StatusPill agent={c.agent} />
                    <span class="hint">{repoName(c.agent)} · {agoText(c.agent.statusSince)}</span>
                  </a>
                {:else if c.kind === 'task'}
                  <a class="card tk" href={href(`/agent/${c.agent.name}`, { s: 'tasks' })}>
                    <span class="hint mono">{c.agent.name} · {c.task.id}</span>
                    <span>{c.task.title}</span>
                    {#if c.task.blockedReason}<span class="hint">Blocked: {c.task.blockedReason}</span>{/if}
                  </a>
                {:else}
                  <a class="card tk" class:needs={c.todo.status === 'due' || c.todo.status === 'error'} href={href('/todos')}>
                    <span>{c.todo.title}</span>
                    <span class="hint">{c.todo.status === 'due' ? 'due' : c.todo.status === 'error' ? 'check failed' : c.todo.status === 'checking' ? 'checking…' : c.todo.condition ? 'next check' : 'due'} {new Date(c.todo.due).toLocaleString([], { dateStyle: 'short', timeStyle: 'short' })}</span>
                  </a>
                {/if}
              {/each}
            </section>
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<style>
  .lane { display: flex; flex-direction: column; gap: 8px; margin-bottom: 18px; }
  .lh { font-size: 14px; margin: 0; padding-bottom: 4px; border-bottom: 1px solid var(--fence-soft); }
  /* One row in flow order; it scrolls sideways when the columns do not fit
     (on a phone, one column and a bit of the next). */
  .cols { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(180px, 1fr); gap: 12px; align-items: start; overflow-x: auto; padding-bottom: 6px; }
  @media (max-width: 760px) {
    .cols { grid-auto-columns: minmax(230px, 72vw); scroll-snap-type: x mandatory; }
    .col { scroll-snap-align: start; }
  }
  .col { display: flex; flex-direction: column; gap: 8px; min-height: 40px; background: var(--pen-2); border-radius: var(--r-card); padding: 8px; }
  .col h3 { font-size: 13px; margin: 0; padding: 2px 4px; }
  a.tk { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; color: inherit; text-decoration: none; font-size: 13px; }
  .tk:hover { border-color: var(--accent); }
  .tk :global(.pill) { align-self: flex-start; }
  .tk.needs { border-color: var(--gate); box-shadow: inset 3px 0 0 var(--gate); }
</style>
