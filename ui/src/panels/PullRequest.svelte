<script lang="ts">
  import Panel from '../components/Panel.svelte';
  import type { Agent } from '../lib/types';
  import { agoText } from '../lib/format';
  import PrComments from '../components/PrComments.svelte';
  let { agent }: { agent: Agent } = $props();
  const STATE_TONE = { open: 'acc', merged: 'done', closed: '' };
  const STATE_GLYPH = { open: '●', merged: '✓', closed: '○' };
  const CI_TONE = { pending: '', failing: 'halt', passing: 'live' };
  const CI_GLYPH = { pending: '…', failing: '✕', passing: '✓' };
  const safe = $derived(agent.prUrl && /^https:\/\//.test(agent.prUrl) ? agent.prUrl : undefined);
</script>

<div class="stack">
  <Panel title="Pull request">
    {#if agent.prUrl}
      <p class="wrap-any">PR: {#if safe}<a href={safe} target="_blank" rel="noopener noreferrer">{agent.prUrl}</a>{:else}<span class="mono">{agent.prUrl}</span>{/if}</p>
      {#if agent.prState}
        <p class="row">
          <span class="pill {STATE_TONE[agent.prState]}">{STATE_GLYPH[agent.prState]} {agent.prState}</span>
          {#if agent.prCi}<span class="pill {CI_TONE[agent.prCi]}">{CI_GLYPH[agent.prCi]} checks {agent.prCi}</span>{/if}
          {#if agent.prConflict}<span class="pill halt">✕ merge conflict</span>{/if}
          {#if agent.prPolledAt}<span class="hint">checked {agoText(agent.prPolledAt)}</span>{/if}
        </p>
      {:else if agent.status === 'watching'}
        <p class="empty">Waiting for the first check of the pull request.</p>
      {/if}
      <p class="muted">New comments on the pull request wake the agent to triage them; you approve its plan before it changes anything.</p>
    {:else}
      <p class="empty">No pull request yet. The agent opens a draft PR after you approve the ship step.</p>
    {/if}
  </Panel>
  {#if agent.prUrl}<PrComments {agent} />{/if}
</div>
