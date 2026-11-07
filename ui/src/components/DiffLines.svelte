<script lang="ts">
  import type { Line } from '../lib/diff';
  let { lines }: { lines: Line[] } = $props();
  const SIGN = { a: '+', d: '−', c: ' ', h: '' };
</script>

<div class="code" role="table" aria-label="Diff">
  {#each lines as l, i (i)}
    <div class="ln {l.kind === 'c' ? '' : l.kind}" role="row">
      <b>{l.old ?? ''}</b><b>{l.new ?? ''}</b><i aria-label={l.kind === 'a' ? 'added' : l.kind === 'd' ? 'removed' : undefined}>{SIGN[l.kind]}</i><span>{l.text}</span>
    </div>
  {/each}
</div>
