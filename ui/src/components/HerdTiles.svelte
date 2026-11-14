<script lang="ts">
  // Herd view "Tiles": a small example of a view (see lib/herdViews.ts). One
  // tile per agent, grouped by repository, finished agents left out. It does
  // not use HerdSummary: a view owns everything below the page head.
  import AgentDot from './AgentDot.svelte';
  import StatusPill from './StatusPill.svelte';
  import { href } from '../lib/router.svelte';
  import { ago, needsYou, repoName } from '../lib/format';
  import { stageNow } from '../lib/flow';
  import type { HerdViewProps } from '../lib/herdViews';

  let { agents, flows, open }: HerdViewProps = $props();

  const groups = $derived.by(() => {
    const by = new Map<string, typeof agents>();
    for (const a of agents) {
      if (a.status === 'done' || a.status === 'cancelled') continue;
      const r = repoName(a);
      by.set(r, [...(by.get(r) ?? []), a]);
    }
    return [...by].sort(([x], [y]) => x.localeCompare(y));
  });
</script>

{#if groups.length}
  <div class="stack">
    {#each groups as [repo, list] (repo)}
      <div class="sect"><h2>{repo}</h2><span class="hint">{list.length}</span></div>
      <div class="tiles">
        {#each list as a (a.id)}
          <a class="card tile" class:need={needsYou(a)} href={href(`/agent/${a.name}`)}
            onclick={(e) => { e.preventDefault(); open(a); }}>
            <div class="row" style="flex-wrap:nowrap">
              <AgentDot agent={a} /><span class="mono name">{a.name}</span>
            </div>
            <StatusPill agent={a} />
            <div class="hint">{stageNow(a, flows)?.name ?? 'no stage'} · {ago(a.lastActivity)}</div>
          </a>
        {/each}
      </div>
    {/each}
  </div>
{:else}
  <p class="empty">No active agents.</p>
{/if}

<style>
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 10px; }
  .tile { display: flex; flex-direction: column; gap: 6px; padding: 12px; color: var(--ink); text-decoration: none; }
  .tile:hover { border-color: var(--fence-strong); }
  .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
