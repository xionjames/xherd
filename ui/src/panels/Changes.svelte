<script lang="ts">
  // `diff` panel: what the user reviews before approving (GET /agents/:id/diff):
  // the proposed commit, the branch at a ship gate, else every uncommitted change.
  import type { Agent, DiffAnswer } from '../lib/types';
  import { get } from '../lib/api';
  import { parseUnified } from '../lib/diff';
  import DiffLines from '../components/DiffLines.svelte';

  let { agent }: { agent: Agent } = $props();
  let d = $state<DiffAnswer | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let collapsed = $state<Record<string, boolean>>({});

  async function load() {
    loading = true;
    try { d = await get<DiffAnswer>(`/agents/${agent.id}/diff`); error = null; }
    catch (e) { error = (e as Error).message; }
    finally { loading = false; }
  }
  // Reload when what is reviewed changes (gate, commits, activity).
  const key = $derived(`${agent.id}|${JSON.stringify(agent.gate ?? null)}|${agent.lastActivity}`);
  $effect(() => { void key; load(); });

  const files = $derived(d ? parseUnified(d.diff) : []);
  const what = $derived(
    !d ? '' : d.commit ? `Commit ${d.commit.id}: the proposed files against HEAD`
      : d.branch ? `The branch as it will be pushed (${d.branch})`
      : 'Every uncommitted change',
  );
</script>

<div class="body">
  <div class="colmain">
    {#if error}
      <div class="card cb muted">{error}</div>
    {:else if d}
      <div class="row">
        <h2>{what}</h2>
        <span class="grow"></span>
        <span class="mono muted">HEAD {d.head.slice(0, 8)}</span>
        <button class="btn sm" onclick={load} disabled={loading}>Refresh</button>
      </div>
      {#if !files.length}<div class="card cb muted">No changes.</div>{/if}
      {#each files as f (f.path)}
        <section class="card">
          <div class="ch">
            <button class="btn ghost sm" aria-expanded={!collapsed[f.path]} onclick={() => (collapsed[f.path] = !collapsed[f.path])}>
              {collapsed[f.path] ? '▸' : '▾'}
            </button>
            <span class="mono">{f.path}</span>
            {#if f.status !== 'modified'}<span class="pill">{f.status}</span>{/if}
            <span class="grow"></span>
            <span class="num" style="color:var(--live)">+{f.adds}</span><span class="num" style="color:var(--halt)">−{f.dels}</span>
          </div>
          {#if !collapsed[f.path]}<DiffLines lines={f.lines} />{/if}
        </section>
      {/each}
    {:else}
      <div class="card cb muted">Loading…</div>
    {/if}
  </div>
  {#if d && (d.files.length || d.others.length)}
    <aside class="colside">
      {#if d.files.length}
        <section class="card cb">
          <span class="lbl">{d.commit ? 'In this commit' : 'Files'} · {d.files.length}</span>
          <ul class="mono" style="font-size:12px">{#each d.files as f (f)}<li>{f}</li>{/each}</ul>
        </section>
      {/if}
      {#if d.others.length && (d.commit || d.branch)}
        <section class="card cb">
          <span class="lbl">{d.branch ? 'Uncommitted, not pushed' : 'Left out of this commit'} · {d.others.length}</span>
          <ul class="mono muted" style="font-size:12px">{#each d.others as f (f)}<li>{f}</li>{/each}</ul>
        </section>
      {/if}
    </aside>
  {/if}
</div>
