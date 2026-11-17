<script lang="ts">
  // The counters above every Jobs layout: Needs you · Running · Waiting · Done.
  // Each is a filter (`?f=`); the layout and the grouping stay as they are.
  import { href, route } from '../lib/router.svelte';
  import { GROUPS, GROUP_LABEL, jobGroup, waitingDetail, isFilter, type JobGroup } from '../lib/jobs';
  import type { Job } from '../lib/types';
  let { agents }: { agents: Job[] } = $props();
  const by = $derived(Object.fromEntries(GROUPS.map((g) => [g, agents.filter((a) => jobGroup(a) === g)])) as Record<JobGroup, Job[]>);
  const f = $derived(isFilter(route.query.get('f')) ? route.query.get('f')! : 'all');
  // Clicking the active counter clears the filter.
  const to = (g: JobGroup) => href('/', { f: f === g ? undefined : g, layout: route.query.get('layout') ?? undefined, group: route.query.get('group') ?? undefined });
</script>

<div class="ctr">
  {#each GROUPS as g (g)}
    <a class="cnt" class:g={g === 'needs' && by[g].length > 0} class:on={f === g} href={to(g)}>
      <b>{by[g].length}</b>{GROUP_LABEL[g]}
      {#if g === 'waiting' && by.waiting.length}<span class="sub">{waitingDetail(by.waiting)}</span>{/if}
    </a>
  {/each}
</div>
