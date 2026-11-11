<script lang="ts">
  // Rendered Markdown, one block at a time, each with a comment box (#25).
  // Notes are keyed `b<index>`; `onnote` also gets the block's source as a quote.
  // Mermaid blocks are drawn as diagrams once the blocks are on the page.
  import { tick } from 'svelte';
  import { drawDiagrams } from '../lib/mermaid';
  import { isDark } from '../lib/theme.svelte';
  let { blocks, notes, onnote }: {
    blocks: { html: string; raw: string }[];
    notes?: Record<string, { text: string; quote: string }>;
    onnote?: (key: string, text: string | null, quote: string) => void;
  } = $props();
  let editing = $state<string | null>(null);
  let root = $state<HTMLElement>();
  $effect(() => {
    void blocks; void isDark();
    const el = root;
    if (el) tick().then(() => drawDiagrams(el));
  });
  let draft = $state('');
  const quoteOf = (raw: string) => { const l = raw.split('\n').find((x) => x.trim()) ?? ''; return l.length > 100 ? l.slice(0, 100) + '…' : l; };
  function start(k: string) { editing = k; draft = notes?.[k]?.text ?? ''; }
  function save(raw: string) {
    if (editing) onnote?.(editing, draft.trim() || null, quoteOf(raw));
    editing = null;
  }
</script>

<div bind:this={root} class="mdb">
{#each blocks as b, i (i)}
  {@const k = `b${i}`}
  <div class="blk">
    {@html b.html}
    {#if onnote}<button class="btn ghost sm bc" aria-label="Comment on this block" onclick={() => start(k)}>💬</button>{/if}
  </div>
  {#if editing === k}
    <div class="lnote flush">
      <textarea bind:value={draft} aria-label="Your comment" placeholder="Comment for the agent…"></textarea>
      <div class="row">
        <button class="btn sm" onclick={() => save(b.raw)}>{draft.trim() ? 'Save' : 'Remove'}</button>
        <button class="btn ghost sm" onclick={() => (editing = null)}>Cancel</button>
      </div>
    </div>
  {:else if notes?.[k]}
    <div class="lnote saved flush">
      <span class="pre">{notes[k].text}</span>
      <button class="btn ghost sm" onclick={() => start(k)}>Edit</button>
    </div>
  {/if}
{/each}
</div>
