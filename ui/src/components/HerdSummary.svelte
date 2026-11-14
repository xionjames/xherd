<script lang="ts">
  // What every Herd view shows above its own body (spec §17.2): the status
  // counts, slot usage, "Needs you" (oldest first) and the filter (`?f=`).
  // The filtered agents, newest activity first, go to the view's `children`.
  import type { Snippet } from 'svelte';
  import NeedsCards from './NeedsCards.svelte';
  import Seg from './Seg.svelte';
  import { app } from '../lib/store.svelte';
  import { route, go } from '../lib/router.svelte';
  import { needsYou } from '../lib/format';
  import type { Agent } from '../lib/types';

  let { agents, children }: { agents: Agent[]; children: Snippet<[Agent[]]> } = $props();

  const waiting = $derived(agents.filter(needsYou).sort((a, b) => a.statusSince - b.statusSince));
  const count = (f: (a: Agent) => boolean) => agents.filter(f).length;
  const slots = $derived(Number(app.state?.config.maxConcurrentAgents ?? 0));
  const busy = $derived(count((a) => ['running', 'waiting-input', 'blocked', 'idle'].includes(a.status)));

  const FILTERS: [string, string, (a: Agent) => boolean][] = [
    ['all', 'All', () => true],
    ['needs', 'Needs you', needsYou],
    ['running', 'Running', (a) => a.status === 'running'],
    ['queued', 'Queued', (a) => a.status === 'queued'],
    ['watching', 'Watching', (a) => a.status === 'watching'],
    ['done', 'Done', (a) => a.status === 'done' || a.status === 'cancelled'],
  ];
  const filter = $derived(route.query.get('f') ?? 'all');
  // Switching the filter keeps the layout.
  const setFilter = (id: string) =>
    go('/', { f: id === 'all' ? undefined : id, layout: route.query.get('layout') ?? undefined });
  const shown = $derived(
    agents
      .filter(FILTERS.find((f) => f[0] === filter)?.[2] ?? (() => true))
      .sort((a, b) => b.lastActivity - a.lastActivity),
  );
</script>

<div class="stack">
  <div class="row">
    {#if waiting.length}<span class="pill gate">✋ {waiting.length} need you</span>{/if}
    {#if count((a) => a.status === 'running')}<span class="pill live">● {count((a) => a.status === 'running')} running</span>{/if}
    {#if count((a) => a.status === 'queued')}<span class="pill">{count((a) => a.status === 'queued')} queued</span>{/if}
    {#if count((a) => a.status === 'watching')}<span class="pill done">{count((a) => a.status === 'watching')} watching PR</span>{/if}
    {#if count((a) => a.status === 'done')}<span class="pill">{count((a) => a.status === 'done')} done</span>{/if}
    <span class="grow"></span>
    {#if slots}<span class="muted num">{busy} of {slots} slots busy</span>{/if}
  </div>

  {#if waiting.length}
    <div class="sect"><h2>Needs you</h2><span class="hint">oldest first</span></div>
    <NeedsCards agents={waiting} />
  {/if}

  <div class="row">
    <Seg label="Filter" value={filter} onchange={setFilter}
      options={FILTERS.map(([id, label, f]) => ({ value: id, label: `${label} ${count(f)}` }))} />
  </div>

  {@render children(shown)}
</div>
