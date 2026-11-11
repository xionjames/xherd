<script lang="ts">
  import Panel from '../components/Panel.svelte';
  // `log` panel: the agent's recent events (GET /agents/:id).
  import type { Agent, AgentEvent } from '../lib/types';
  import { get } from '../lib/api';
  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  let events = $state<AgentEvent[]>([]);
  $effect(() => {
    void agent.lastActivity;
    void agent.status;
    get<{ events: AgentEvent[] }>(`/agents/${id}`).then((r) => (events = r.events));
  });
  const time = (ms: number) => new Date(ms).toLocaleString();
  const summary = (p: unknown) => {
    if (p == null) return '';
    const s = typeof p === 'string' ? p : JSON.stringify(p);
    return s.length > 160 ? s.slice(0, 160) + '…' : s;
  };
</script>

<div class="stack">
  <Panel title="Activity" flush>
    {#snippet head()}<span class="hint">latest first</span>{/snippet}
    {#if events.length}
      <div class="tscroll">
        <table class="tbl">
          <tbody>
            {#each events as e (e.id)}
              <tr><td class="num muted" style="white-space:nowrap">{time(e.at)}</td><td class="mono">{e.kind}</td><td class="mono muted hide-sm" style="word-break:break-all">{summary(e.payload)}</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="cb"><p class="empty">Nothing yet.</p></div>
    {/if}
  </Panel>
</div>
