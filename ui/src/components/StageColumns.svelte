<script lang="ts">
  // Herd view "By stage" (R-210, #142): the summary, then a lane per workflow
  // the shown agents use (no heading when there is one), each with that
  // workflow's stage columns in flow order, plus Not started, Watching PR and
  // Finished (`lib/stageLanes.ts`). The count in each header shows where
  // agents pile up. Cards outlined with --gate need you.
  import HerdSummary from './HerdSummary.svelte';
  import Panel from './Panel.svelte';
  import StatusPill from './StatusPill.svelte';
  import TicketPill from './TicketPill.svelte';
  import { href } from '../lib/router.svelte';
  import { stageLanes } from '../lib/stageLanes';
  import { agoText, needsYou, repoName } from '../lib/format';
  import type { HerdViewProps } from '../lib/herdViews';
  import type { Agent } from '../lib/types';

  let { agents, flows, open }: HerdViewProps = $props();

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
      {#each stageLanes(shown, flows) as lane (lane.key)}
        <section class="lane" aria-label={lane.label || 'By stage'}>
          {#if lane.label}<h2 class="lh">{lane.label} <span class="hint num">{lane.cols.reduce((n, c) => n + c.agents.length, 0)}</span></h2>{/if}
          <div class="cols">
            {#each lane.cols as col (col.key)}
              <section class="col" aria-label={col.label}>
                <h3>{col.label} <span class="hint num">{col.agents.length}</span></h3>
                {#each col.agents as a (a.id)}
                  <a class="card tk" class:needs={needsYou(a)} href={href(`/agent/${a.name}`)} onclick={(e) => click(e, a)}>
                    <span class="mono"><b>{a.name}</b> <TicketPill ticket={a.ticket} link={false} /></span>
                    <StatusPill agent={a} />
                    <span class="hint">{repoName(a)} · {agoText(a.statusSince)}</span>
                  </a>
                {/each}
              </section>
            {/each}
          </div>
        </section>
      {/each}
    {:else}
      <Panel title="All agents">
        <p class="empty">{#if agents.length}No agents match this filter.{:else}No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p>
      </Panel>
    {/if}
  {/snippet}
</HerdSummary>

<style>
  .lane { display: flex; flex-direction: column; gap: 8px; margin-bottom: 18px; }
  .lane:last-child { margin-bottom: 0; }
  .lh { font-size: 14px; margin: 0; padding-bottom: 4px; border-bottom: 1px solid var(--fence-soft); }
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
