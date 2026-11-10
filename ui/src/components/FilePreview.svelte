<script lang="ts">
  // Preview of a changed file as it is in the worktree now (GET /agents/:id/file).
  // Agent-written content is untrusted: Markdown goes through lib/markdown.ts,
  // HTML into an iframe with an empty sandbox (no scripts, no same-origin),
  // images as <img> from a data URL (scripts in SVG do not run there).
  import { get } from '../lib/api';
  import { render, frontmatter } from '../lib/markdown';
  import MdBlocks from './MdBlocks.svelte';
  import type { MdNote } from '../lib/notes';

  let { id, path, kind, notes, onnote }: {
    id: string; path: string; kind: 'markdown' | 'html' | 'image';
    notes?: Record<string, MdNote>; onnote?: (key: string, text: string | null, quote: string) => void;
  } = $props();
  let text = $state('');
  let dataUrl = $state<string | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const p = path; // read before the await so the effect depends on it
    get<{ text: string; dataUrl: string | null }>(`/agents/${id}/file?path=${encodeURIComponent('./' + p)}`)
      .then((r) => { text = r.text; dataUrl = r.dataUrl; error = null; })
      .catch((e) => { error = (e as Error).message; });
  });
</script>

{#if error}
  <div class="cb muted">{error}</div>
{:else if kind === 'markdown'}
  <div class="cb md"><MdBlocks blocks={render(frontmatter(text).body).blocks} {notes} {onnote} /></div>
{:else if kind === 'html'}
  <iframe title="Preview of {path}" sandbox="" srcdoc={text} style="width:100%;height:480px;border:0;background:#fff"></iframe>
{:else if dataUrl}
  <div class="cb"><img src={dataUrl} alt={path} style="max-width:100%" /></div>
{/if}
