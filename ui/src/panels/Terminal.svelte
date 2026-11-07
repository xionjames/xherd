<script lang="ts">
  // `terminal` panel: the agent's tmux session through /ws/term/:id.
  import TermView from '../components/TermView.svelte';
  import type { Agent } from '../lib/types';

  let { agent }: { agent: Agent } = $props();
  // Keyed on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const live = $derived(['running', 'waiting-input', 'blocked', 'idle'].includes(agent.status));
</script>

<div class="stack">
  {#if live}
    <TermView path={`/ws/term/${id}`} title={agent.name}
      hint={`tmux session ${agent.name}; leaving this page detaches, the agent keeps running.`} />
  {:else}
    <section class="term" aria-label="Terminal">
      <div class="scr c-dim">No session: the agent is {agent.status}. Resume it to get a terminal.</div>
    </section>
  {/if}
</div>
