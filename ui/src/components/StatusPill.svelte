<script lang="ts">
  import type { Agent } from '../lib/types';
  import { needsYou, waitLabel, STATUS_LABEL } from '../lib/format';
  let { agent }: { agent: Agent } = $props();
  const tone = $derived(
    needsYou(agent) ? 'gate'
      : agent.status === 'running' ? 'live'
      : agent.status === 'error' ? 'halt'
      : agent.status === 'watching' ? 'done' : '',
  );
</script>

<span class="pill {tone}">
  {#if tone === 'gate'}✋ {waitLabel(agent)}
  {:else if tone === 'live'}<span class="dot" style="background:var(--live);width:6px;height:6px"></span>{STATUS_LABEL[agent.status]}
  {:else if tone === 'halt'}✕ {STATUS_LABEL[agent.status]}
  {:else}{STATUS_LABEL[agent.status]}{/if}
</span>
