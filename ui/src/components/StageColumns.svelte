<script lang="ts">
  // Herd view "By stage" (R-210): the summary, then one card per agent in the
  // column of its current stage's role, in flow order, plus Not started,
  // Watching PR and Finished. The count in each header shows where agents
  // pile up. Cards outlined with --gate need you.
  import HerdSummary from './HerdSummary.svelte';
  import Panel from './Panel.svelte';
  import StatusPill from './StatusPill.svelte';
  import TicketPill from './TicketPill.svelte';
  import { href } from '../lib/router.svelte';
  import { flowOf, STAGE_LABEL } from '../lib/flow';
  import { agoText, needsYou, repoName } from '../lib/format';
  import type { HerdViewProps } from '../lib/herdViews';
  import type { Agent } from '../lib/types';

  let { agents, flows, open }: HerdViewProps = $props();

  const ROLE_ORDER = ['explore', 'spec', 'design', 'tasks', 'implement', 'review', 'ship', 'pr'];
  const NOT_STARTED = 'start';
  const WATCHING = 'watching';
  const FINISHED = 'finished';

  /** The column of an agent's current stage: its role, so a column looks the
   *  same in every pack. */
  function stageCol(a: Agent): string {
    if (a.status === 'watching') return WATCHING;
    if (a.status === 'done' || a.status === 'cancelled') return FINISHED;
    const stage = a.stage ? flowOf(a, flows)?.stages.find((s) => s.id === a.stage) : undefined;
    return stage ? stage.role : NOT_STARTED;
  }

  function columns(placed: { a: Agent; col: string }[]): [string, string][] {
    const used = new Set(placed.map((p) => p.col));
    const roles = new Set(flows.flatMap((f) => f.stages.map((s) => s.role as string)));
    const cols: [string, string][] = [];
    if (used.has(NOT_STARTED)) cols.push([NOT_STARTED, 'Not started']);
    for (const r of ROLE_ORDER) if (roles.has(r) || used.has(r)) cols.push([r, STAGE_LABEL[r] ?? r]);
    for (const r of used) if (!cols.some(([c]) => c === r) && r !== FINISHED && r !== WATCHING) cols.push([r, r]);
    cols.push([WATCHING, 'Watching PR'], [FINISHED, 'Finished']);
    return cols;
  }

  // A card is a real link; a plain click goes through `open`.
  function click(e: MouseEvent, a: Agent) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    open(a);
  }
</script>

<HerdSummary {agents}>
  {#snippet children(shown: Agent[])}
    {#if shown.length}
      {@const placed = shown.map((a) => ({ a, col: stageCol(a) }))}
      <div class="cols">
        {#each columns(placed) as [col, label] (col)}
          {@const list = placed.filter((p) => p.col === col)}
          <section class="col" aria-label={label}>
            <h3>{label} <span class="hint num">{list.length}</span></h3>
            {#each list as { a } (a.id)}
              <a class="card tk" class:needs={needsYou(a)} href={href(`/agent/${a.name}`)} onclick={(e) => click(e, a)}>
                <span class="mono"><b>{a.name}</b> <TicketPill ticket={a.ticket} link={false} /></span>
                <StatusPill agent={a} />
                <span class="hint">{repoName(a)} · {agoText(a.statusSince)}</span>
              </a>
            {/each}
          </section>
        {/each}
      </div>
    {:else}
      <Panel title="All agents">
        <p class="empty">{#if agents.length}No agents match this filter.{:else}No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p>
      </Panel>
    {/if}
  {/snippet}
</HerdSummary>

<style>
  .cols { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(170px, 1fr); gap: 12px; align-items: start; overflow-x: auto; }
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
