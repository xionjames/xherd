<script lang="ts">
  import Icon from './Icon.svelte';
  import type { Job, FlowInfo } from '../lib/types';
  import { steps, stageLabel } from '../lib/flow';
  import { stageRuns } from '../lib/agents';
  let { agent, flow }: { agent: Job; flow?: FlowInfo } = $props();
  // Each stage names its agent when the job has any (D26, R42).
  const runs = $derived(flow ? stageRuns(agent, flow.stages) : []);
  const named = $derived(runs.some((r) => r.agent));
</script>

{#if flow}
  <ol class="stepper" aria-label="Progress">
    {#each steps(agent, flow) as s, i (s.stage.id)}
      {#if i > 0}<li class="wire" class:ok={s.state !== 'todo'} aria-hidden="true"></li>{/if}
      <li class="st {s.state === 'todo' ? '' : s.state}" aria-current={s.state === 'now' || s.state === 'cur' ? 'step' : undefined}>
        <i>{#if s.state === 'ok'}✓{:else if s.state === 'now'}<Icon name="handSolid" size={18} solid />{:else if s.state === 'skip'}–{:else}{i + 1}{/if}</i>
        {stageLabel(s.stage)}
        {#if named && runs[i]?.agent}<span class="t mono" title={runs[i].human ? 'Its gate always waits for you' : 'The agent that runs this stage'}>{runs[i].agent}{runs[i].human ? ' ✋' : ''}</span>{/if}
        {#if s.state === 'now'}<span class="t">waiting for you</span>{:else if s.state === 'cur' && agent.status === 'running'}<span class="t">running</span>{:else if s.state === 'skip'}<span class="t">skipped</span>{/if}
      </li>
    {/each}
  </ol>
{:else}
  <div><span class="pill">No workflow</span></div>
{/if}
