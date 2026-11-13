<script lang="ts">
  import type { Agent } from '../lib/types';
  import { needsYou, STATUS_LABEL } from '../lib/format';
  let { agent }: { agent: Agent } = $props();
  const tone = $derived(
    needsYou(agent) ? 'gate'
      : agent.status === 'running' ? 'live'
      : agent.status === 'error' ? 'halt'
      : agent.status === 'watching' ? 'done' : '',
  );
  // The state only; where it waits (stage, document, commit) is shown apart.
  const wait = $derived(
    agent.status === 'blocked' ? 'blocked'
      : agent.gate && agent.gate.kind !== 'question' ? 'needs you'
      : 'your answer',
  );
</script>

<span class="pill {tone}">
  {#if tone === 'gate'}✋ {wait}
  {:else if tone === 'live'}<span class="dot" style="background:var(--live);width:6px;height:6px"></span>{STATUS_LABEL[agent.status]}
  {:else if tone === 'halt'}✕ {STATUS_LABEL[agent.status]}
  {:else}{STATUS_LABEL[agent.status]}{/if}
</span>
