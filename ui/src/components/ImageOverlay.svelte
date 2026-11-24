<script lang="ts">
  // A document image at full size, in the app's own dialog (R5).
  import Dialog from './Dialog.svelte';
  let { src, alt, onclose }: { src: string; alt: string; onclose: () => void } = $props();
</script>

<Dialog title={alt || 'Image'} {onclose}>
  <img class="ovimg" {src} {alt} />
  {#snippet actions()}<button class="btn" onclick={onclose}>Close</button>{/snippet}
</Dialog>
