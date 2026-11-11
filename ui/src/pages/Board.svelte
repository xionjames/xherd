<script lang="ts">
  // Board (spec §17): every unit of work by state, Pending / In progress /
  // Done, plus your TODOs. A unit is a flow stage (spec, design, tasks,
  // review, ship…), or, once an agent has a task list, each of its tasks in
  // place of the implement stage. Fix tasks from a review or the PR carry a
  // tag. Where an agent waits for you the card is outlined. Read-only. Filter
  // and swimlanes live in the URL: `?agent=NAME&lanes=agent|repo|none`.
  import PageHead from '../components/PageHead.svelte';
  import { app } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { get } from '../lib/api';
  import { flowOf, steps, STAGE_LABEL } from '../lib/flow';
  import { needsYou, repoName } from '../lib/format';
  import type { Agent, StageInfo, Task } from '../lib/types';

  type Todo = { id: string; title: string; due: number; status: 'open' | 'checking' | 'due' | 'done' | 'error'; condition?: string };
  type Col = 'pending' | 'doing' | 'done';
  type Card =
    | { kind: 'stage'; key: string; lane: string; col: Col; agent: Agent; label: string; tab: string; hint: string; needs: boolean }
    | { kind: 'task'; key: string; lane: string; col: Col; agent: Agent; task: Task; needs: boolean }
    | { kind: 'todo'; key: string; lane: string; col: 'todos'; todo: Todo };

  const lanes = $derived((['repo', 'none'] as const).find((l) => l === route.query.get('lanes')) ?? 'agent');
  const only = $derived(route.query.get('agent') ?? '');
  const go = (q: Record<string, string | undefined>) => {
    location.hash = href('/board', { agent: route.query.get('agent') ?? undefined, lanes: route.query.get('lanes') ?? undefined, ...q });
  };

  let todos = $state<Todo[]>([]);
  $effect(() => {
    const load = () => get<{ todos: Todo[] }>('/todos').then((r) => (todos = r.todos.filter((t) => t.status !== 'done'))).catch(() => {});
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  });

  const COLS: [Col, string][] = [['pending', 'Pending'], ['doing', 'In progress'], ['done', 'Done']];
  const TASK_COL: Record<Task['status'], Col> = { todo: 'pending', doing: 'doing', blocked: 'doing', done: 'done' };
  const SOURCE_TAG: Record<string, string> = { review: 'review fix', pr: 'PR fix' };

  // Cancelled agents are left out: their work is neither pending nor done.
  const everyone = $derived((app.state?.agents ?? []).filter((a) => a.status !== 'cancelled'));
  const shown = $derived(everyone.filter((a) => !only || a.name === only));
  const laneOf = (a: Agent) => (lanes === 'repo' ? repoName(a) : lanes === 'agent' ? a.name : '');

  /** "Spec", "PR"…; a stage whose id is not its role names itself by its id,
   *  known words as their labels (`pr-review` → "PR Review"). */
  function label(s: StageInfo): string {
    if (s.id === s.role) return STAGE_LABEL[s.role] ?? s.role;
    return s.id.split(/[-_]/).map((w) => STAGE_LABEL[w] ?? w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
  }
  /** The agent tab that shows a stage's work. */
  function stageTab(s: StageInfo): string {
    if (s.role === 'implement') return 'tasks';
    if (s.role === 'pr') return 'pr';
    return s.gate ? 'spec' : 'activity';
  }

  function agentCards(a: Agent): Card[] {
    const lane = laneOf(a);
    const waits = needsYou(a);
    const flow = flowOf(a, app.state?.flows ?? []);
    if (!flow) {
      // Flow `none`: the agent is the one unit of work.
      const col: Col = a.status === 'done' || a.status === 'watching' ? 'done' : a.status === 'queued' ? 'pending' : 'doing';
      return [{ kind: 'stage', key: a.id, lane, col, agent: a, label: a.ticketKey || a.name, tab: 'terminal', hint: 'no workflow', needs: waits }];
    }
    const tasks = a.tasks ?? [];
    const gateTasks = a.gate?.kind === 'commit' ? a.gate.tasks : [];
    const out: Card[] = [];
    for (const { stage, state } of steps(a, flow)) {
      const current = stage.id === a.stage;
      if (stage.role === 'implement' && tasks.length) {
        // The task list replaces the implement stage. A commit gate marks its
        // tasks; any other wait at this stage marks the tasks being worked on.
        for (const t of tasks) {
          const needs = current && waits && (gateTasks.length ? gateTasks.includes(t.id) : t.status === 'doing');
          out.push({ kind: 'task', key: `${a.id}/${t.id}`, lane, col: TASK_COL[t.status], agent: a, task: t, needs });
        }
        continue;
      }
      const col: Col = state === 'ok' || state === 'skip' ? 'done' : state === 'todo' ? 'pending' : 'doing';
      // A later stage already visited: the agent was sent back (review → fix).
      const again = state === 'todo' && (a.visitedStages ?? []).includes(stage.id);
      const hint = state === 'skip' ? 'skipped' : again ? 'again after the fixes' : current && a.gate ? 'waiting for you' : '';
      out.push({ kind: 'stage', key: `${a.id}:${stage.id}`, lane, col, agent: a, label: label(stage), tab: stageTab(stage), hint, needs: current && waits });
    }
    // Fix tasks of a later stage (review, PR) when the implement stage had none.
    if (!flow.stages.some((s) => s.role === 'implement')) {
      for (const t of tasks) out.push({ kind: 'task', key: `${a.id}/${t.id}`, lane, col: TASK_COL[t.status], agent: a, task: t, needs: false });
    }
    return out;
  }

  const cards = $derived.by((): Card[] => {
    const out = shown.flatMap(agentCards);
    if (!only) for (const t of todos) out.push({ kind: 'todo', key: `todo:${t.id}`, lane: lanes === 'none' ? '' : 'TODOs', col: 'todos', todo: t });
    return out;
  });
  const columns = $derived<[string, string][]>(cards.some((c) => c.kind === 'todo') ? [...COLS, ['todos', 'TODOs']] : COLS);
  const laneNames = $derived([...new Set(cards.map((c) => c.lane))].sort((a, b) => (a === 'TODOs' ? 1 : b === 'TODOs' ? -1 : a.localeCompare(b))));
</script>

<PageHead title="Board">
  {#snippet actions()}
    <select aria-label="Swimlanes" value={lanes} onchange={(e) => go({ lanes: e.currentTarget.value === 'agent' ? undefined : e.currentTarget.value })}>
      <option value="agent">Lanes by agent</option><option value="repo">Lanes by repository</option><option value="none">No swimlanes</option>
    </select>
    <select aria-label="Agent" value={only} onchange={(e) => go({ agent: e.currentTarget.value || undefined })}>
      <option value="">All agents</option>
      {#each everyone as a (a.id)}<option value={a.name}>{a.name}</option>{/each}
    </select>
  {/snippet}
</PageHead>

<div class="stack">
  {#if !cards.length}
    <p class="empty">No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.</p>
  {:else}
    {#each laneNames as lane (lane)}
      <section class="lane" aria-label={lane || 'Board'}>
        {#if lane}<h2 class="lh">{lane}</h2>{/if}
        <div class="cols">
          {#each columns as [col, name] (col)}
            {@const list = cards.filter((c) => c.lane === lane && c.col === col)}
            <section class="col" aria-label={name}>
              <h3>{name} <span class="hint num">{list.length}</span></h3>
              {#each list as c (c.key)}
                {#if c.kind === 'stage'}
                  <a class="card tk" class:needs={c.needs} href={href(`/agent/${c.agent.name}`, { s: c.tab })}>
                    {#if lanes !== 'agent'}<span class="hint mono">{c.agent.name}</span>{/if}
                    <span><b>{c.label}</b></span>
                    {#if c.needs}<span class="hint">✋ {c.hint || 'needs you'}</span>{:else if c.hint}<span class="hint">{c.hint}</span>{/if}
                  </a>
                {:else if c.kind === 'task'}
                  <a class="card tk" class:needs={c.needs} href={href(`/agent/${c.agent.name}`, { s: 'tasks' })}>
                    <span class="top"><span class="hint mono">{lanes === 'agent' ? '' : `${c.agent.name} · `}{c.task.id}</span>{#if c.task.source && SOURCE_TAG[c.task.source]}<span class="pill">{SOURCE_TAG[c.task.source]}</span>{/if}</span>
                    <span>{c.task.title}</span>
                    {#if c.task.status === 'blocked'}<span class="hint">⛔ Blocked{c.task.blockedReason ? `: ${c.task.blockedReason}` : ''}</span>{/if}
                    {#if c.needs}<span class="hint">✋ needs you</span>{/if}
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
  /* Columns side by side; they scroll sideways when they do not fit (on a
     phone, one column and a bit of the next). */
  .cols { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(180px, 1fr); gap: 12px; align-items: start; overflow-x: auto; padding-bottom: 6px; }
  @media (max-width: 760px) {
    .cols { grid-auto-columns: minmax(230px, 72vw); scroll-snap-type: x mandatory; }
    .col { scroll-snap-align: start; }
  }
  .col { display: flex; flex-direction: column; gap: 8px; min-height: 40px; background: var(--pen-2); border-radius: var(--r-card); padding: 8px; }
  .col h3 { font-size: 13px; margin: 0; padding: 2px 4px; }
  a.tk { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; color: inherit; text-decoration: none; font-size: 13px; }
  .tk:hover { border-color: var(--accent); }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .tk.needs { border-color: var(--gate); box-shadow: inset 3px 0 0 var(--gate); }
</style>
