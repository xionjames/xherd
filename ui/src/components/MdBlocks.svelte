<script lang="ts">
  // Rendered Markdown, one block at a time, each with a comment box (#25).
  // Notes are keyed `b<index>`; `onnote` also gets the block's source as a quote.
  // Mermaid blocks are drawn as diagrams once the blocks are on the page.
  // `questions` (block indexes, R-105a) keep their answer box open: what is
  // typed there is the note, no Save step.
  // Local images arrive as `img[data-xh-img]` with no `src` (nothing is requested
  // by parsing): `load` fetches each one, `reload` (any value) loads them again.
  // The object URLs are revoked when their image leaves the page and on destroy.
  import { onDestroy, tick } from 'svelte';
  import { drawDiagrams } from '../lib/mermaid';
  import { isDark } from '../lib/theme.svelte';
  import { reasonOf } from '../lib/images';
  import ImageOverlay from './ImageOverlay.svelte';
  let { blocks, notes, onnote, questions = [], load, reload }: {
    blocks: { html: string; raw: string }[];
    notes?: Record<string, { text: string; quote: string }>;
    onnote?: (key: string, text: string | null, quote: string) => void;
    questions?: number[];
    load?: (path: string) => Promise<{ url: string; blob: string }>;
    reload?: unknown;
  } = $props();
  let editing = $state<string | null>(null);
  let root = $state<HTMLElement>();
  let zoom = $state<{ src: string; alt: string } | null>(null);
  const urls = new Map<HTMLImageElement, { url: string; round: unknown }>();
  const revoke = (url: string) => { if (url.startsWith('blob:')) URL.revokeObjectURL(url); };
  function loadImages(el: HTMLElement, round: unknown) {
    for (const [n, v] of urls) if (!el.contains(n)) { revoke(v.url); urls.delete(n); }
    if (!load) return;
    for (const img of el.querySelectorAll<HTMLImageElement>('img[data-xh-img]')) {
      if (urls.get(img)?.round === round) continue;
      const path = img.dataset.xhImg ?? '';
      load(path).then(
        ({ url }) => {
          if (!el.contains(img)) return revoke(url);
          const old = urls.get(img);
          if (old) revoke(old.url);
          urls.set(img, { url, round });
          img.src = url;
          img.tabIndex = 0;
        },
        (e) => {
          const ph = document.createElement('span');
          ph.className = 'imgph';
          ph.textContent = `${img.alt || path} — ${reasonOf(e)}`;
          img.replaceWith(ph);
          urls.delete(img);
        },
      );
      urls.set(img, { url: urls.get(img)?.url ?? '', round });
    }
  }
  $effect(() => {
    void blocks; void isDark();
    const el = root;
    const round = reload;
    if (el) tick().then(() => { drawDiagrams(el); loadImages(el, round); });
  });
  onDestroy(() => { for (const v of urls.values()) revoke(v.url); urls.clear(); });
  function open(e: Event) {
    const t = e.target as Element;
    if (t instanceof HTMLImageElement && t.src && t.closest('.blk')) zoom = { src: t.src, alt: t.alt };
  }
  let draft = $state('');
  const quoteOf = (raw: string) => { const l = raw.split('\n').find((x) => x.trim()) ?? ''; return l.length > 100 ? l.slice(0, 100) + '…' : l; };
  function start(k: string) { editing = k; draft = notes?.[k]?.text ?? ''; }
  function save(raw: string) {
    if (editing) onnote?.(editing, draft.trim() || null, quoteOf(raw));
    editing = null;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div bind:this={root} class="mdb" onclick={open} onkeydown={(e) => e.key === 'Enter' && open(e)}>
{#each blocks as b, i (i)}
  {@const k = `b${i}`}
  {@const asks = !!onnote && questions.includes(i)}
  <div class="blk" class:qblk={asks}>
    {@html b.html}
    {#if onnote && !asks}<button class="btn ghost sm bc" aria-label="Comment on this block" onclick={() => start(k)}>💬</button>{/if}
  </div>
  {#if asks}
    <div class="lnote flush">
      <textarea rows="2" value={notes?.[k]?.text ?? ''} aria-label={`Answer: ${quoteOf(b.raw)}`} placeholder="Your answer"
        oninput={(e) => { const v = e.currentTarget.value; onnote?.(k, v.trim() ? v : null, quoteOf(b.raw)); }}></textarea>
    </div>
  {:else if editing === k}
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
{#if zoom}<ImageOverlay src={zoom.src} alt={zoom.alt} onclose={() => (zoom = null)} />{/if}
