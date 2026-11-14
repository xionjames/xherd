<script lang="ts">
  // Herd view "List": the summary, then every agent in a table.
  import AgentDot from './AgentDot.svelte';
  import HerdSummary from './HerdSummary.svelte';
  import Mini from './Mini.svelte';
  import TaskBar from './TaskBar.svelte';
  import StatusPill from './StatusPill.svelte';
  import TicketPill from './TicketPill.svelte';
  import Panel from './Panel.svelte';
  import { href } from '../lib/router.svelte';
  import { ago, repoName, shortBase } from '../lib/format';
  import { stageNow } from '../lib/flow';
  import type { Agent, FlowInfo } from '../lib/types';

  let { agents, flows, open }: { agents: Agent[]; flows: FlowInfo[]; open: (a: Agent) => void } = $props();
</script>

<HerdSummary {agents}>
  {#snippet children(shown: Agent[])}
    <Panel title="All agents" flush>
      {#if shown.length}
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Agent</th><th>Status</th><th>Stage</th><th class="hide-sm">Progress</th><th class="hide-sm">Tasks</th><th class="hide-sm">Branch</th><th class="r">Active</th></tr></thead>
            <tbody>
              {#each shown as a (a.id)}
                <tr class="hov" onclick={() => open(a)}>
                  <td>
                    <div class="row" style="flex-wrap:nowrap">
                      <AgentDot agent={a} />
                      <div class="wrap-any">
                        <a class="mono nyname" href={href(`/agent/${a.name}`)}>{a.name}</a> <TicketPill ticket={a.ticket} />
                        <div class="hint">{repoName(a)} · {a.flow ?? 'no workflow'}</div>
                      </div>
                    </div>
                  </td>
                  <td><StatusPill agent={a} /></td>
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
      {:else}
        <div class="cb">
          <p class="empty">{#if agents.length}No agents match this filter.{:else}No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p>
        </div>
      {/if}
    </Panel>
  {/snippet}
</HerdSummary>

<style>
  .nyname { font-weight: 600; color: var(--ink); }
</style>
