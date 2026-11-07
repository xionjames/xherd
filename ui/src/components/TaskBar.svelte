<script lang="ts">
  import type { Agent } from '../lib/types';
  let { agent }: { agent: Agent } = $props();
  const t = $derived(agent.tasks ?? []);
  const n = (s: string) => t.filter((x) => x.status === s).length;
</script>

{#if t.length}
  <span class="row" style="gap:8px">
    <span class="bar4" aria-hidden="true">
      <i style="flex:{n('done')};background:var(--live)"></i>
      <i style="flex:{n('doing')};background:var(--accent)"></i>
      <i style="flex:{n('blocked')};background:var(--halt)"></i>
      <i style="flex:{n('todo')};background:var(--fence-strong)"></i>
    </span>
    <span class="num muted">{n('done')}/{t.length}</span>
  </span>
{:else}
  <span class="muted">—</span>
{/if}
