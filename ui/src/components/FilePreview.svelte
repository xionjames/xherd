<script lang="ts">
  // Preview of a changed Markdown or HTML file: its new side in the diff being
  // reviewed (GET /jobs/:id/diff/source, R7), not only the worktree.
  // Agent-written content is untrusted: Markdown goes through lib/markdown.ts,
  // HTML into an iframe with an empty sandbox (no scripts, no same-origin).
  // Images of the Markdown load from the same side of the diff (R7c).
  // Images files themselves are shown by ImageSide.
  import { get, getImage } from '../lib/api';
  import { render, frontmatter } from '../lib/markdown';
  import { fileBase } from '../lib/images';
  import MdBlocks from './MdBlocks.svelte';
  import type { MdNote } from '../lib/notes';
  import type { DiffSource } from '../lib/types';

  let { id, path, kind, query = '', notes, onnote }: {
    id: string; path: string; kind: 'markdown' | 'html';
    /** The diff's query (`?view=…`), or ''. */
    query?: string;
    notes?: Record<string, MdNote>; onnote?: (key: string, text: string | null, quote: string) => void;
  } = $props();
  let text = $state('');
  let error = $state<string | null>(null);

  $effect(() => {
    const [i, p, q] = [id, path, query]; // read before the await so the effect depends on them
    get<DiffSource>(`/jobs/${i}/diff/source${q ? `${q}&` : '?'}path=${encodeURIComponent(p)}`)
      .then((r) => { text = r.text; error = null; })
      .catch((e) => { error = (e as Error).message; });
  });
  const loadImage = (p: string) =>
    getImage(`/jobs/${id}/image?path=${encodeURIComponent(p)}&side=new${query ? `&${query.slice(1)}` : ''}`);
</script>

{#if error}
  <div class="cb muted">{error}</div>
{:else if kind === 'markdown'}
  <div class="cb md"><MdBlocks blocks={render(frontmatter(text).body, undefined, false, fileBase(path)).blocks} {notes} {onnote} load={loadImage} reload={text} /></div>
{:else}
  <iframe title="Preview of {path}" sandbox="" srcdoc={text} style="width:100%;height:480px;border:0;background:#fff"></iframe>
{/if}
