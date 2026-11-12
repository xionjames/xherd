<script lang="ts">
  import type { Line } from '../lib/diff';
  // With `notes` + `onnote`, a line number becomes a button that opens a
  // comment box under the line (inline review, #25), and so does the 💬 that
  // shows on hover, as on rendered Markdown. Notes are keyed by `lineKey`.
  let { lines, notes, onnote }: {
    lines: Line[];
    notes?: Record<string, string>;
    onnote?: (key: string, text: string | null) => void;
  } = $props();
  const SIGN = { a: '+', d: '−', c: ' ', h: '' };
  let editing = $state<string | null>(null);
  let draft = $state('');
  const commentable = (l: Line) => !!onnote && l.kind !== 'h';
  function start(l: Line) {
    const k = lineKey(l);
    editing = k;
    draft = notes?.[k] ?? '';
  }
  function save() {
    if (editing) onnote?.(editing, draft.trim() || null);
    editing = null;
  }
</script>

<script lang="ts" module>
  export const lineKey = (l: Line) => (l.kind === 'd' ? `o${l.old}` : `n${l.new}`);
</script>

<div class="code" role="table" aria-label="Diff">
  {#each lines as l, i (i)}
    <div class="ln {l.kind === 'c' ? '' : l.kind}" role="row">
      <b>{l.old ?? ''}</b>
      <b>{#if commentable(l)}<button class="lnc" aria-label="Comment on this line" onclick={() => start(l)}>{l.new ?? l.old}</button>{:else}{l.new ?? ''}{/if}</b>
      <i aria-label={l.kind === 'a' ? 'added' : l.kind === 'd' ? 'removed' : undefined}>{SIGN[l.kind]}</i><span>{l.text}</span>
      {#if commentable(l)}<button class="btn ghost sm bc" aria-label="Comment on this line" tabindex="-1" onclick={() => start(l)}>💬</button>{/if}
    </div>
    {#if commentable(l)}
      {@const k = lineKey(l)}
      {#if editing === k}
        <div class="lnote">
          <textarea bind:value={draft} aria-label="Your comment" placeholder="Comment for the agent…"></textarea>
          <div class="row">
            <button class="btn sm" onclick={save}>{draft.trim() ? 'Save' : 'Remove'}</button>
            <button class="btn ghost sm" onclick={() => (editing = null)}>Cancel</button>
          </div>
        </div>
      {:else if notes?.[k]}
        <div class="lnote saved">
          <span class="pre">{notes[k]}</span>
          <button class="btn ghost sm" onclick={() => start(l)}>Edit</button>
        </div>
      {/if}
    {/if}
  {/each}
</div>
