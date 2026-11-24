<script lang="ts">
  // One side (Before or After) of an image in a diff (R7, R7b, R7c, R7e, R9).
  // The image is fetched once the section is near the viewport. The blob id the
  // route serves is always compared with the diff's (`expect`): a side that
  // changed since the diff was loaded is not shown.
  import { onDestroy } from 'svelte';
  import { getImage } from '../lib/api';
  import { reasonOf, stale } from '../lib/images';
  import ImageOverlay from './ImageOverlay.svelte';

  let { id, path, side, query = '', label, expect, quiet = false }: {
    id: string; path: string; side: 'old' | 'new';
    /** The diff's query (`?view=…`), or ''. */
    query?: string; label?: string;
    /** The diff's abbreviated blob id of this side. */
    expect?: string;
    /** Show nothing when the side does not exist. */
    quiet?: boolean;
  } = $props();

  let host = $state<HTMLElement>();
  let near = $state(false);
  let shown = $state<string | null>(null);
  let msg = $state<string | null>(null);
  let zoom = $state(false);
  let current = '';
  const revoke = (url: string) => { if (url.startsWith('blob:')) URL.revokeObjectURL(url); };
  const drop = () => { revoke(current); current = ''; };

  $effect(() => {
    const el = host;
    if (!el || near) return;
    if (typeof IntersectionObserver === 'undefined') { near = true; return; }
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) { near = true; io.disconnect(); }
    }, { rootMargin: '200px' });
    io.observe(el);
    return () => io.disconnect();
  });

  $effect(() => {
    if (!near) return;
    const [i, p, s, q, e] = [id, path, side, query, expect];
    let live = true;
    getImage(`/jobs/${i}/image?path=${encodeURIComponent('./' + p)}&side=${s}${q ? `&${q.slice(1)}` : ''}`).then(
      ({ url, blob }) => {
        if (!live) return revoke(url);
        drop();
        if (stale(e, blob)) {
          revoke(url);
          shown = null;
          msg = 'The file changed since this diff was loaded: refresh.';
          return;
        }
        current = url; shown = url; msg = null;
      },
      (err) => {
        if (!live) return;
        drop();
        shown = null;
        const why = reasonOf(err);
        msg = quiet && why === 'not found' ? null : `${p} — ${why}`;
      },
    );
    return () => { live = false; };
  });
  onDestroy(drop);
</script>

<figure class="imgside" bind:this={host} class:hidden={quiet && !shown && !msg && near}>
  {#if label}<figcaption class="lbl">{label}</figcaption>{/if}
  {#if shown}
    <button class="imgbtn" aria-label="Open {path} at full size" onclick={() => (zoom = true)}>
      <img class="mdimg" src={shown} alt={path} />
    </button>
  {:else if msg}
    <span class="imgph">{msg}</span>
  {:else if !near}
    <span class="muted">Loads when it comes into view.</span>
  {:else if !quiet}
    <span class="muted">Loading…</span>
  {/if}
</figure>
{#if zoom && shown}<ImageOverlay src={shown} alt={path} onclose={() => (zoom = false)} />{/if}
