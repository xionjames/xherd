<script lang="ts">
  // `log` panel: the agent's recent events (GET /agents/:id).
  import type { Agent, AgentEvent } from '../lib/types';
  import { get } from '../lib/api';
  let { agent }: { agent: Agent } = $props();
  let events = $state<AgentEvent[]>([]);
  $effect(() => {
    void agent.lastActivity;
    void agent.status;
    get<{ events: AgentEvent[] }>(`/agents/${agent.id}`).then((r) => (events = r.events));
  });
  const time = (ms: number) => new Date(ms).toLocaleString();
  const summary = (p: unknown) => {
    if (p == null) return '';
    const s = typeof p === 'string' ? p : JSON.stringify(p);
    return s.length > 160 ? s.slice(0, 160) + '…' : s;
  };
</script>

<div class="stack">
  <section class="card">
    <div class="ch"><h2>Activity</h2><span class="muted">latest first</span></div>
    {#if events.length}
      <table class="tbl">
        <tbody>
          {#each events as e (e.id)}
            <tr><td class="num muted" style="white-space:nowrap">{time(e.at)}</td><td class="mono">{e.kind}</td><td class="mono muted" style="word-break:break-all">{summary(e.payload)}</td></tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="cb muted">Nothing yet.</div>
    {/if}
  </section>
</div>
