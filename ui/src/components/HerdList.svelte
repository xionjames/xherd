<script lang="ts">
  // Herd view "List": the summary, then every agent in a table.
  import AgentDot from './AgentDot.svelte';
  import HerdSummary from './HerdSummary.svelte';
  import Mini from './Mini.svelte';
  import TaskBar from './TaskBar.svelte';
  import StatusPill from './StatusPill.svelte';
  import TicketPill from './TicketPill.svelte';
  import Panel from './Panel.svelte';
  import { href, route, go } from '../lib/router.svelte';
  import Seg from './Seg.svelte';
  import { app } from '../lib/store.svelte';
  import { agentLane, agentLaneOrder, waitsForAgent } from '../lib/agents';
  import { ago, repoName, shortBase } from '../lib/format';
  import { stageNow } from '../lib/flow';
  import type { Job, FlowInfo } from '../lib/types';

  let { agents, flows, open }: { agents: Job[]; flows: FlowInfo[]; open: (a: Job) => void } = $props();

  // `?group=agent`: one table per saved agent (the job's snapshot), jobs
  // without one last (R23).
  const byAgent = $derived(route.query.get('group') === 'agent');
  const setGroup = (g: string) =>
    go('/', { f: route.query.get('f') ?? undefined, layout: route.query.get('layout') ?? undefined, group: g === 'agent' ? 'agent' : undefined });
  function groups(shown: Job[]): { name: string; jobs: Job[] }[] {
    if (!byAgent) return [{ name: '', jobs: shown }];
    const names = agentLaneOrder(shown.map(agentLane));
    return names.map((name) => ({ name, jobs: shown.filter((j) => agentLane(j) === name) }));
  }
</script>

<HerdSummary {agents}>
  {#snippet children(shown: Job[])}
    <div class="row">
      <Seg label="Group by" value={byAgent ? 'agent' : 'none'} onchange={setGroup}
        options={[{ value: 'none', label: 'No grouping' }, { value: 'agent', label: 'By agent' }]} />
    </div>
    {#if byAgent && shown.length}
      {#each groups(shown) as g (g.name)}
        <Panel title={g.name} flush>{@render table(g.jobs)}</Panel>
      {/each}
    {:else}
    <Panel title="All jobs" flush>
      {#if shown.length}
        {@render table(shown)}
      {:else}
        <div class="cb">
          <p class="empty">{#if agents.length}No jobs match this filter.{:else}No jobs yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p>
        </div>
      {/if}
    </Panel>
    {/if}
  {/snippet}
</HerdSummary>

{#snippet table(shown: Job[])}
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Job</th><th>Status</th><th>Stage</th><th class="hide-sm">Progress</th><th class="hide-sm">Tasks</th><th class="hide-sm">Branch</th><th class="r">Active</th></tr></thead>
            <tbody>
              {#each shown as a (a.id)}
                <tr class="hov" onclick={() => open(a)}>
                  <td>
                    <div class="row" style="flex-wrap:nowrap">
                      <AgentDot agent={a} />
                      <div class="wrap-any">
                        <a class="mono nyname" href={href(`/jobs/${a.name}`)}>{a.name}</a> <TicketPill ticket={a.ticket} />
                        <div class="hint">{repoName(a)} · {a.flow ?? 'no workflow'}{#if a.agent && !byAgent} · agent {a.agent.name}{/if}</div>
                      </div>
                    </div>
                  </td>
                  <td><StatusPill agent={a} />{#if waitsForAgent(a, app.state?.agents ?? [])}<div class="hint" title="Its agent already has as many live jobs as it may">waits for its agent</div>{/if}</td>
                  <td>
                    {#if stageNow(a, flows)}
                      {@const st = stageNow(a, flows)!}
                      {st.name}{#if st.waits}<span class="muted">{' · '}<span class="mono">{st.waits}</span></span>{/if}
                    {:else}<span class="muted">–</span>{/if}
                  </td>
                  <td class="hide-sm"><Mini agent={a} /></td>
                  <td class="hide-sm"><TaskBar agent={a} /></td>
                  <td class="hide-sm mono muted">{a.workdir ? `${a.branch} ← ${shortBase(a)}` : 'not created'}</td>
                  <td class="num muted r">{ago(a.lastActivity)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
{/snippet}

<style>
  .nyname { font-weight: 600; color: var(--ink); }
</style>
