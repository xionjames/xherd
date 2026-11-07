<script lang="ts">
  // Overview: "Needs you" (oldest first) then every agent (spec §17.2).
  import AgentDot from '../components/AgentDot.svelte';
  import Mini from '../components/Mini.svelte';
  import TaskBar from '../components/TaskBar.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import { app } from '../lib/store.svelte';
  import { href, route, go } from '../lib/router.svelte';
  import { ago, needsYou, waitText, repoName, shortBase } from '../lib/format';
  import type { Agent } from '../lib/types';

  const agents = $derived(app.state?.agents ?? []);
  const waiting = $derived(agents.filter(needsYou).sort((a, b) => a.statusSince - b.statusSince));
  const count = (f: (a: Agent) => boolean) => agents.filter(f).length;
  const slots = $derived(Number(app.state?.config.maxConcurrentAgents ?? 0));
  const busy = $derived(count((a) => ['running', 'waiting-input', 'blocked', 'idle'].includes(a.status)));

  const FILTERS: [string, string, (a: Agent) => boolean][] = [
    ['all', 'All', () => true],
    ['needs', 'Needs you', needsYou],
    ['running', 'Running', (a) => a.status === 'running'],
    ['queued', 'Queued', (a) => a.status === 'queued'],
    ['watching', 'Watching', (a) => a.status === 'watching'],
    ['done', 'Done', (a) => a.status === 'done' || a.status === 'cancelled'],
  ];
  const filter = $derived(route.query.get('f') ?? 'all');
  const shown = $derived(
    agents
      .filter(FILTERS.find((f) => f[0] === filter)?.[2] ?? (() => true))
      .sort((a, b) => b.lastActivity - a.lastActivity),
  );

  /** The tab that holds the decision for a waiting agent. */
  function decisionTab(a: Agent): string {
    const g = a.gate;
    if (!g) return 'terminal';
    if (g.kind === 'commit') return 'changes';
    if (g.kind === 'question') return 'terminal';
    if (g.kind === 'docs' && g.role === 'ship') return 'changes';
    if (g.kind === 'docs') return 'spec';
    return 'pr';
  }
  function actionLabel(a: Agent): string {
    const g = a.gate;
    if (!g || g.kind === 'question') return 'Answer';
    if (g.kind === 'commit') return 'Review changes';
    if (g.kind === 'docs') return 'Review';
    return 'Decide';
  }
  function meta(a: Agent): string {
    const g = a.gate;
    const parts = [repoName(a)];
    if (g?.kind === 'commit') parts.push(`${g.files.length} files`);
    if (g?.kind === 'docs' && g.questions?.length) parts.push(`${g.questions.length} open questions`);
    if (g && 'needsHuman' in g && g.needsHuman) parts.push(`held: ${g.needsHuman}`);
    if (!g) parts.push('asked in the terminal');
    return parts.join(' · ');
  }
</script>

<div class="phead notabs">
  <div class="row" style="gap:14px">
    <h1>Herd</h1>
    {#if waiting.length}<span class="pill gate">{waiting.length} need you</span>{/if}
    {#if count((a) => a.status === 'running')}<span class="pill live">● {count((a) => a.status === 'running')} running</span>{/if}
    {#if count((a) => a.status === 'queued')}<span class="pill">{count((a) => a.status === 'queued')} queued</span>{/if}
    {#if count((a) => a.status === 'watching')}<span class="pill done">{count((a) => a.status === 'watching')} watching PR</span>{/if}
    {#if count((a) => a.status === 'done')}<span class="pill">{count((a) => a.status === 'done')} done</span>{/if}
    <span class="grow"></span>
    {#if slots}<span class="muted num">{busy} of {slots} slots busy</span>{/if}
  </div>
</div>

<div class="stack">
  {#if waiting.length}
    <section class="row" style="gap:8px" aria-labelledby="ny-h">
      <h2 id="ny-h" style="color:var(--gate)">Needs you</h2><span class="hint">oldest first</span>
    </section>
    <div class="nygrid">
      {#each waiting as a (a.id)}
        <article class="ny">
          <div class="row">
            <AgentDot agent={a} /><a class="mono" style="font-weight:600;color:var(--ink)" href={href(`/agent/${a.name}`)}>{a.name}</a>
            <span class="grow"></span><span class="muted num">{ago(a.statusSince)}</span>
          </div>
          <p class:qtext={a.gate?.kind === 'question'} class:mono={a.gate?.kind === 'commit'}>{waitText(a)}</p>
          <p class="muted" style="font-size:12px">{meta(a)}</p>
          <div class="row" style="justify-content:flex-end">
            <button class="btn gate" onclick={() => go(`/agent/${a.name}`, { s: decisionTab(a) })}>{actionLabel(a)}</button>
          </div>
        </article>
      {/each}
    </div>
  {/if}

  <section class="card">
    <div class="ch">
      <h2>All agents</h2>
      <div class="seg" role="group" aria-label="Filter">
        {#each FILTERS as [id, label, f] (id)}
          <button class:on={filter === id} aria-pressed={filter === id}
            onclick={() => go('/', { f: id === 'all' ? undefined : id })}>{label} {count(f)}</button>
        {/each}
      </div>
    </div>
    {#if shown.length}
      <table class="tbl">
        <thead><tr><th>Agent</th><th>Status</th><th>Progress</th><th>Tasks</th><th>Branch</th><th style="text-align:right">Active</th></tr></thead>
        <tbody>
          {#each shown as a (a.id)}
            <tr class="hov" onclick={() => go(`/agent/${a.name}`)} style="cursor:pointer">
              <td>
                <div class="row" style="gap:8px;flex-wrap:nowrap">
                  <AgentDot agent={a} />
                  <div>
                    <a class="mono" style="color:var(--ink)" href={href(`/agent/${a.name}`)}>{a.name}</a>
                    <div class="muted" style="font-size:12px">{repoName(a)} · {a.flow ?? 'no workflow'}</div>
                  </div>
                </div>
              </td>
              <td><StatusPill agent={a} /></td>
              <td><Mini agent={a} /></td>
              <td><TaskBar agent={a} /></td>
              <td class="mono muted">{a.workdir ? `← ${shortBase(a)}` : 'not created'}</td>
              <td class="num muted" style="text-align:right">{ago(a.lastActivity)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="cb muted">
        {#if agents.length}No agents match this filter.{:else}No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}
      </div>
    {/if}
  </section>
</div>

<style>
  .nygrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 14px; }
  .qtext { color: var(--gate); }
</style>
