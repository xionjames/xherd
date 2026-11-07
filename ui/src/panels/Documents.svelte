<script lang="ts">
  // `documents` panel: the agent's spec documents, rendered as paper, with
  // "changed since you approved" against the last approved text (R-107).
  import type { Agent } from '../lib/types';
  import { get } from '../lib/api';
  import { frontmatter, render } from '../lib/markdown';
  import { lineDiff } from '../lib/diff';
  import { markRead } from '../lib/reads.svelte';
  import { route, go } from '../lib/router.svelte';
  import DiffLines from '../components/DiffLines.svelte';

  let { agent }: { agent: Agent } = $props();

  interface Doc { path: string; text: string; sha256: string; reviewed: string | null }
  let docs = $state<string[]>([]);
  let doc = $state<Doc | null>(null);
  let error = $state<string | null>(null);
  let showDiff = $state(false);

  const gateFiles = $derived(agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.path) : []);
  const selected = $derived(route.query.get('doc') ?? docs[0]);
  // Reload when the gate's hashes change (the agent rewrote a document).
  const gateKey = $derived(agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.sha256).join() : '');

  $effect(() => {
    void gateKey;
    get<{ docs: string[] }>(`/agents/${agent.id}/docs`).then((r) => (docs = r.docs), (e) => (error = e.message));
  });
  $effect(() => {
    void gateKey;
    const path = selected;
    if (!path) return;
    get<Doc>(`/agents/${agent.id}/file?path=${encodeURIComponent(path)}`).then(
      (d) => { doc = d; error = null; markRead(agent.id, d.path, d.sha256); },
      (e) => { doc = null; error = e.message; },
    );
  });

  const parsed = $derived(doc ? frontmatter(doc.text) : null);
  const rendered = $derived(parsed ? render(parsed.body) : null);
  const changed = $derived(!!doc?.reviewed && doc.reviewed !== doc.text);
</script>

<div class="body">
  <div class="colmain">
    {#if docs.length}
      <div class="row">
        {#each docs as d (d)}
          <button class="chip" class:on={d === selected} aria-pressed={d === selected}
            onclick={() => go(`/agent/${agent.name}`, { s: 'spec', doc: d })}>
            {d}{#if gateFiles.includes(d)} ✋{/if}
          </button>
        {/each}
        <span class="grow"></span>
        {#if changed}
          <span style="color:var(--accent)">Changed since you approved</span>
          <button class="btn sm" onclick={() => (showDiff = !showDiff)}>{showDiff ? 'Show document' : 'Show changes'}</button>
        {:else if doc?.reviewed}
          <span class="pill live">✓ approved version</span>
        {/if}
      </div>
    {/if}
    {#if error}
      <div class="card cb muted">{error}</div>
    {:else if !docs.length}
      <div class="card cb muted">No documents yet.</div>
    {:else if doc && showDiff && changed}
      <div class="card"><DiffLines lines={lineDiff(doc.reviewed ?? '', doc.text)} /></div>
    {:else if parsed && rendered}
      <article class="paper">
        {#if parsed.meta.length}
          <div class="row" style="margin-bottom:12px">
            {#each parsed.meta as [k, v] (k)}<span class="pill">{k}: {v}</span>{/each}
          </div>
        {/if}
        <div class="md">{@html rendered.html}</div>
      </article>
    {/if}
  </div>
  {#if rendered?.headings.length}
    <aside class="colside">
      <nav class="card cb outline" aria-label="On this page">
        <span class="lbl">On this page</span>
        <div>
          {#each rendered.headings.filter((h) => h.level <= 2) as h (h.id)}
            <a href={`#${route.path}?${route.query}`} onclick={(e) => { e.preventDefault(); document.getElementById(h.id)?.scrollIntoView({ behavior: 'smooth' }); }}>{h.text}</a>
          {/each}
        </div>
      </nav>
    </aside>
  {/if}
</div>

<style>
  .md :global(pre) { background: var(--pen-2); padding: 10px 12px; border-radius: 8px; overflow-x: auto; font: 12px/1.55 var(--mono); }
  .md :global(code) { font-family: var(--mono); font-size: 12.5px; }
  .md :global(table) { border-collapse: collapse; }
  .md :global(th), .md :global(td) { border: 1px solid var(--fence); padding: 4px 8px; }
  .md :global(h3) { font-size: 13px; margin: 6px 0 0; }
  .md :global(p), .md :global(ul), .md :global(ol) { margin: 0; }
</style>
