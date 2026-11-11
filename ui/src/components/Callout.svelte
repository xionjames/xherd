<script lang="ts">
  // A status line that must be noticed: a glyph and the text in full ink
  // (never coloured text), with the colour on the edge and the background.
  // `need` is for what waits on the user, like --gate.
  import type { Snippet } from 'svelte';
  type Tone = 'ok' | 'bad' | 'info' | 'need' | 'plain';
  const GLYPH: Record<Tone, string> = { ok: '✓', bad: '✕', info: 'i', need: '✋', plain: '•' };
  let { tone = 'plain', glyph, class: cls = '', children }: { tone?: Tone; glyph?: string; class?: string; children: Snippet } = $props();
</script>

<div class="callout {tone} {cls}" role="status">
  <span class="ci {tone}" aria-hidden="true">{glyph ?? GLYPH[tone]}</span>
  <div>{@render children()}</div>
</div>
