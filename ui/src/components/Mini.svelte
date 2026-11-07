<script lang="ts">
  // One dot per stage: teal done, coral waiting, violet running, grey to do.
  import type { Agent } from '../lib/types';
  import { app } from '../lib/store.svelte';
  import { flowOf, steps, stageLabel } from '../lib/flow';
  let { agent }: { agent: Agent } = $props();
  const f = $derived(flowOf(agent, app.state?.flows ?? []));
  const COLOR = { ok: 'var(--live)', now: 'var(--gate)', cur: 'var(--accent)', todo: 'var(--fence-strong)', skip: 'transparent' };
</script>

{#if f}
  <span class="row" style="gap:4px" role="img"
    aria-label={steps(agent, f).map((s) => `${stageLabel(s.stage)} ${s.state}`).join(', ')}>
    {#each steps(agent, f) as s (s.stage.id)}
      <i style="width:8px;height:8px;border-radius:50%;background:{COLOR[s.state]};{s.state === 'skip' ? 'box-shadow:inset 0 0 0 1.5px var(--live)' : ''}"></i>
    {/each}
  </span>
{:else}
  <span class="muted">no workflow</span>
{/if}
