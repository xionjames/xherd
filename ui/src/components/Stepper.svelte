<script lang="ts">
  import type { Agent, FlowInfo } from '../lib/types';
  import { steps, stageLabel } from '../lib/flow';
  let { agent, flow }: { agent: Agent; flow?: FlowInfo } = $props();
</script>

{#if flow}
  <ol class="stepper" aria-label="Progress">
    {#each steps(agent, flow) as s, i (s.stage.id)}
      {#if i > 0}<li class="wire" class:ok={s.state !== 'todo'} aria-hidden="true"></li>{/if}
      <li class="st {s.state === 'todo' ? '' : s.state}" aria-current={s.state === 'now' || s.state === 'cur' ? 'step' : undefined}>
        <i>{#if s.state === 'ok'}✓{:else if s.state === 'now'}✋{:else if s.state === 'skip'}–{:else}{i + 1}{/if}</i>
        {stageLabel(s.stage)}
        {#if s.state === 'now'}<span class="t">waiting for you</span>{:else if s.state === 'cur' && agent.status === 'running'}<span class="t">running</span>{:else if s.state === 'skip'}<span class="t">skipped</span>{/if}
      </li>
    {/each}
  </ol>
{:else}
  <div><span class="pill">No workflow</span></div>
{/if}
