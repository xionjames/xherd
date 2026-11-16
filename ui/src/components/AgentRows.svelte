<script lang="ts">
  // Herd view "By agent" (D26, R43): a row per saved agent with the jobs whose
  // session runs it now; agents with jobs first, `No agent` last. `?agent=NAME`
  // shows one agent, `?archived=1` adds archived agents (`lib/herdAgents.ts`).
  import HerdSummary from './HerdSummary.svelte';
  import Panel from './Panel.svelte';
  import StatusPill from './StatusPill.svelte';
  import TicketPill from './TicketPill.svelte';
  import { app } from '../lib/store.svelte';
  import { route, href, go } from '../lib/router.svelte';
  import { agentRows } from '../lib/herdAgents';
  import { agoText, needsYou, repoName, agentHue } from '../lib/format';
  import type { HerdViewProps } from '../lib/herdViews';
  import type { Job } from '../lib/types';

  let { agents, open }: HerdViewProps = $props();

  const only = $derived(route.query.get('agent'));
  const archived = $derived(route.query.get('archived') === '1');
  const saved = $derived(app.state?.agents ?? []);
  const q = (over: Record<string, string | undefined>) => {
    const cur: Record<string, string | undefined> = {};
    route.query.forEach((v, k) => { cur[k] = v; });
    return Object.assign(cur, over);
  };
  const hasArchived = $derived(saved.some((a) => a.status === 'archived'));

  function click(e: MouseEvent, a: Job) {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    open(a);
  }
</script>

<HerdSummary {agents}>
  {#snippet children(shown: Job[])}
    {@const rows = agentRows(shown, saved, { agent: only, archived })}
    <div class="bar">
      {#if only}<a class="btn ghost" href={href('/', q({ agent: undefined }))}>All agents</a>{/if}
      {#if hasArchived}
        <button class="btn ghost" aria-pressed={archived} onclick={() => go('/', q({ archived: archived ? undefined : '1' }))}>{archived ? 'Hide' : 'Show'} archived</button>
      {/if}
    </div>
    {#if rows.length}
      {#each rows as row (row.key)}
        <Panel title={row.label} hint={row.agent ? `${row.jobs.length} job${row.jobs.length === 1 ? '' : 's'} · v${row.agent.version}${row.agent.status === 'archived' ? ' · archived' : ''}` : `${row.jobs.length} job${row.jobs.length === 1 ? '' : 's'}`}>
          {#if row.jobs.length}
            <div class="cards">
              {#each row.jobs as a (a.id)}
                <a class="card tk" class:needs={needsYou(a)} href={href(`/jobs/${a.name}`)} onclick={(e) => click(e, a)}>
                  <span class="mono"><span class="hue" style:background={agentHue(a.name)}></span><b>{a.name}</b> <TicketPill ticket={a.ticket} link={false} /></span>
                  <StatusPill agent={a} />
                  <span class="hint">{repoName(a)} · {agoText(a.statusSince)}</span>
                </a>
              {/each}
            </div>
          {:else}
            <p class="empty">No jobs run this agent now.
              {#if row.agent && row.agent.status !== 'archived'}<a href={href('/new', { agent: row.agent.name })}>Start one</a>.{/if}</p>
          {/if}
          {#if row.agent && !only}<p class="hint"><a href={href('/', q({ agent: row.label }))}>Only this agent</a> · <a href={href(`/agents/${row.label}`)}>Agent page</a></p>{/if}
        </Panel>
      {/each}
    {:else}
      <Panel title="By agent">
        <p class="empty">{#if only}No agent named {only}.{:else}No agents or jobs yet. <a href={href('/agents/new')}>Create an agent</a>.{/if}</p>
      </Panel>
    {/if}
  {/snippet}
</HerdSummary>

<style>
  .bar { display: flex; gap: 8px; margin-bottom: 10px; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; }
  a.tk { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; color: inherit; text-decoration: none; font-size: 13px; }
  .tk:hover { border-color: var(--accent); }
  .tk :global(.pill) { align-self: flex-start; }
  .tk.needs { border-color: var(--gate); box-shadow: inset 3px 0 0 var(--gate); }
  .hue { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }
</style>
