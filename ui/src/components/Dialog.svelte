<script lang="ts">
  // In-app dialog (never browser alert/confirm, spec §17.1).
  import type { Snippet } from 'svelte';
  let { title, onclose, children, actions }: { title: string; onclose: () => void; children: Snippet; actions?: Snippet } = $props();
  let box: HTMLDivElement;
  $effect(() => { box?.focus(); });
  // Escape typed into a terminal belongs to the program running there.
  function onkey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || (e.target as Element | null)?.closest?.('.term')) return;
    onclose();
  }
</script>

<svelte:window onkeydown={onkey} />
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dlg" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" bind:this={box}>
    <div class="ch"><h2>{title}</h2></div>
    <div class="cb">{@render children()}</div>
    {#if actions}<div class="cf">{@render actions()}</div>{/if}
  </div>
</div>
