<script lang="ts">
  // `documents` panel: the agent's spec documents, rendered as paper, with
  // "changed since you approved" against the last approved text (R-107).
  import type { Agent } from '../lib/types';
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { mdNoteText, questionBlocks, type MdNote } from '../lib/notes';
  import { tick } from 'svelte';
  import MdBlocks from '../components/MdBlocks.svelte';
  import { frontmatter, render } from '../lib/markdown';
  import { lineDiff } from '../lib/diff';
  import { markRead } from '../lib/reads.svelte';
  import { route, go } from '../lib/router.svelte';
  import DiffLines from '../components/DiffLines.svelte';
  import Findings from '../components/Findings.svelte';
  import Seg from '../components/Seg.svelte';
  import { prGate } from '../lib/flow';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);

  interface Doc { path: string; text: string; sha256: string; reviewed: string | null }
  let docs = $state<string[]>([]);
  let doc = $state<Doc | null>(null);
  let error = $state<string | null>(null);
  // When the document changed since the user read it, the changes are marked
  // in the page; "Show changes" switches to a line diff.
  let showDiff = $state(false);

  const gateFiles = $derived(agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.path) : []);
  const selected = $derived(route.query.get('doc') ?? docs[0]);
  // Reload when the gate's hashes change (the agent rewrote a document).
  // The PR-comment plan is decided in the Pull request tab, not read here
  // (a string, so the effect below does not rerun on every agent update).
  const hidden = $derived(prGate(agent.gate) ? gateFiles.join('\n') : '');
  const gateKey = $derived(agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.sha256).join() : '');

  $effect(() => {
    void gateKey;
    const hide = hidden.split('\n');
    get<{ docs: string[] }>(`/agents/${id}/docs`).then((r) => (docs = r.docs.filter((d) => !hide.includes(d))), (e) => (error = e.message));
  });
  $effect(() => {
    void gateKey;
    const path = selected;
    if (!path) return;
    get<Doc>(`/agents/${id}/file?path=${encodeURIComponent(path)}`).then(
      (d) => { doc = d; error = null; markRead(id, d.path, d.sha256); },
      (e) => { doc = null; error = e.message; },
    );
  });

  // Inline review comments (#25), per document, sent as one change request.
  let notes = $state<Record<string, Record<string, MdNote>>>({});
  let sending = $state(false);
  const setNote = (key: string, text: string | null, quote: string) => {
    if (!selected) return;
    const n = { ...(notes[selected] ?? {}) };
    if (text) n[key] = { text, quote }; else delete n[key];
    notes[selected] = n;
  };
  // Questions the agent asked outside the documents (R-105a), answered here.
  const asked = $derived(agent.gate?.kind === 'docs' ? (agent.gate.asked ?? []) : []);
  let answers = $state<Record<string, string>>({});
  const answered = $derived(asked.filter((q) => answers[q]?.trim()));
  const count = $derived(Object.values(notes).reduce((a, n) => a + Object.keys(n).length, 0) + answered.length);
  async function sendNotes() {
    const out = [
      ...answered.map((q) => `Q: ${q}\nA: ${answers[q].trim()}`),
      ...Object.entries(notes).flatMap(([p, n]) => mdNoteText(p, n)),
    ];
    sending = true;
    try {
      await post(`/agents/${id}/request-changes`, { comment: `Review comments on the documents:\n\n${out.join('\n\n')}` });
      notes = {};
      answers = {};
    } catch (e) { showError('Could not send the comments', e); }
    finally { sending = false; }
  }
  const parsed = $derived(doc ? frontmatter(doc.text) : null);
  const changed = $derived(!!doc?.reviewed && doc.reviewed !== doc.text);
  const added = $derived(
    new Set(
      changed && doc?.reviewed != null
        ? lineDiff(doc.reviewed, doc.text).filter((l) => l.kind === 'a').map((l) => l.text.trim()).filter(Boolean)
        : [],
    ),
  );
  const rendered = $derived(parsed ? render(parsed.body, added) : null);
  // Open questions of a gate document get their answer boxes (R-105a).
  const questions = $derived(rendered && selected && gateFiles.includes(selected) ? questionBlocks(rendered.blocks) : []);
  // `?q=1` (the band's link) scrolls to the first question.
  $effect(() => {
    if (!route.query.get('q') || !questions.length) return;
    tick().then(() => document.querySelector('.qblk')?.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  });
</script>

<div class="body">
  <div class="colmain">
    {#if docs.length}
      <div class="row">
        <Seg label="Document" value={selected ?? ''} onchange={(d) => go(`/agent/${agent.name}`, { s: 'spec', doc: d })}
          options={docs.map((d) => ({ value: d, label: gateFiles.includes(d) ? `${d} ✋` : d }))} />
        <span class="grow"></span>
        {#if count}
          <button class="btn gate sm" onclick={sendNotes} disabled={sending || !agent.gate}
            title={agent.gate ? '' : 'The agent is not waiting at a gate'}>Send {count} comment{count === 1 ? '' : 's'}</button>
        {/if}
        {#if changed}
          <span class="pill acc">Changed since you last reviewed it</span>
          <button class="btn sm" onclick={() => (showDiff = !showDiff)}>{showDiff ? 'Show document' : 'Show changes'}</button>
        {:else if doc?.reviewed}
          <span class="pill live">✓ as you last reviewed it</span>
        {/if}
      </div>
    {/if}
    {#if error}
      <div class="card cb"><p class="err">{error}</p></div>
    {:else if !docs.length}
      <div class="card cb"><p class="empty">No documents yet.</p></div>
    {:else if doc && showDiff && changed}
      <div class="card"><DiffLines lines={lineDiff(doc.reviewed ?? '', doc.text)} /></div>
    {:else if parsed && rendered}
      {#if doc?.path === 'review.md'}<Findings {agent} />{/if}
      {#if asked.length}
        <div class="card cb asked">
          <span class="lbl">Questions from the agent · not in the document</span>
          {#each asked as q (q)}
            <label class="gbox">
              <span>{q}</span>
              <textarea rows="2" bind:value={answers[q]} aria-label={`Answer: ${q}`} placeholder="Your answer"></textarea>
            </label>
          {/each}
        </div>
      {/if}
      <article class="paper">
        {#if parsed.meta.length}
          <div class="row meta">
            {#each parsed.meta as [k, v] (k)}<span class="pill">{k}: {v}</span>{/each}
          </div>
        {/if}
        <div class="md"><MdBlocks blocks={rendered.blocks} notes={selected ? notes[selected] : undefined} onnote={setNote} {questions} /></div>
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
  .md :global(.chg) { border-left: 3px solid var(--accent); background: color-mix(in srgb, var(--accent) 9%, transparent); padding-left: 10px; margin-left: -13px; border-radius: 2px; }
  .md :global(p), .md :global(ul), .md :global(ol) { margin: 0; }
  .meta { margin-bottom: 12px; }
  .asked { display: grid; gap: 10px; margin-bottom: 12px; }
  .asked .gbox { display: grid; gap: 4px; }
</style>
