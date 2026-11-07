<script lang="ts">
  import type { Agent } from '../lib/types';
  let { agent }: { agent: Agent } = $props();
  const safe = $derived(agent.prUrl && /^https:\/\//.test(agent.prUrl) ? agent.prUrl : undefined);
</script>

<div class="stack">
  <section class="card">
    <div class="ch"><h2>Pull request</h2></div>
    <div class="cb">
      {#if agent.prUrl}
        <p>Draft PR: {#if safe}<a href={safe} target="_blank" rel="noopener noreferrer">{agent.prUrl}</a>{:else}<span class="mono">{agent.prUrl}</span>{/if}</p>
        <p class="muted">Review comments and checks come with PR follow-up.</p>
      {:else}
        <p class="muted">No pull request yet. The agent opens a draft PR after you approve the ship step.</p>
      {/if}
    </div>
  </section>
</div>
