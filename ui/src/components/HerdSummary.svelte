<script lang="ts">
  // What the By stage layout shows above its body: the same counters as the
  // list (`JobCounters`, filter `?f=`) and the slots in use. What waits for
  // you is in the Inbox. The filtered jobs, newest activity first, go to the
  // view's `children`.
  import type { Snippet } from 'svelte';
  import JobCounters from './JobCounters.svelte';
  import { app } from '../lib/store.svelte';
  import { route } from '../lib/router.svelte';
  import { filterJobs, isFilter, type JobFilter } from '../lib/jobs';
  import type { Job } from '../lib/types';

  let { agents, children }: { agents: Job[]; children: Snippet<[Job[]]> } = $props();

  const slots = $derived(Number(app.state?.config.maxConcurrentJobs ?? 0));
  const busy = $derived(agents.filter((a) => ['running', 'waiting-input', 'blocked', 'idle'].includes(a.status)).length);
  const filter = $derived((isFilter(route.query.get("f")) ? route.query.get("f") : "all") as JobFilter);
  const shown = $derived(filterJobs(agents, filter).sort((a, b) => b.lastActivity - a.lastActivity));
</script>

<div class="stack">
  <JobCounters {agents} />
  {#if slots}<div class="row"><span class="grow"></span><span class="muted num">{busy} of {slots} slots busy</span></div>{/if}
  {@render children(shown)}
</div>
