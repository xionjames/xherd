<script lang="ts">
  // The one way to save edited settings: what is unsaved on the left,
  // Discard then Save on the right. Sticky while something is unsaved.
  let { dirty, busy = false, onsave, ondiscard, saveLabel = 'Save changes', saved = 'No unsaved changes' }: {
    dirty: number | boolean;
    busy?: boolean;
    onsave: () => void;
    ondiscard: () => void;
    saveLabel?: string;
    saved?: string;
  } = $props();
  const text = $derived(
    !dirty ? saved : typeof dirty === 'number' ? `${dirty} unsaved change${dirty > 1 ? 's' : ''}` : 'Unsaved changes',
  );
</script>

<div class="savebar" class:dirty={!!dirty}>
  <span class="st-text grow muted" aria-live="polite">{text}</span>
  <button class="btn ghost" disabled={!dirty || busy} onclick={ondiscard}>Discard</button>
  <button class="btn primary" disabled={!dirty || busy} onclick={onsave}>{busy ? 'Saving…' : saveLabel}</button>
</div>
