<script lang="ts">
  // One bar per stage of the job's workflow, the agent's initials under it:
  // green = done, blue = current, coral = waits for you, dashed = skipped.
  import { track, segTitle, avatarBg } from '../lib/jobs';
  import type { Job, FlowInfo } from '../lib/types';
  let { job, flows }: { job: Job; flows: FlowInfo[] } = $props();
  const segs = $derived(track(job, flows));
  const cur = $derived(segs.find((s) => s.state === 'now' || s.state === 'cur'));
</script>

{#if segs.length}
  <div class="trk" role="img" aria-label={cur ? segTitle(cur) : 'stages'}>
    {#each segs as s (s.stage)}
      <span class="stc {s.state}" title={segTitle(s)}><span class="stp {s.state}">{s.label}</span><i>{s.initials}{#if s.fresh} · check{/if}</i></span>
    {/each}
  </div>
{:else}
  <span class="muted">no workflow</span>
{/if}
