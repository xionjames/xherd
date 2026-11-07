<script lang="ts">
  import type { Agent } from '../lib/types';
  import { agentHue, hollow } from '../lib/format';
  let { agent }: { agent: Agent } = $props();
  const c = $derived(agentHue(agent.name));
</script>

<span class="dot" style={hollow(agent.status) ? `border:1.5px solid ${c}` : `background:${c}`}></span>
