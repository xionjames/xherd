<script lang="ts">
  import Dialog from './Dialog.svelte';
  import { errors } from '../lib/errors.svelte';
  const close = () => { errors.title = null; errors.message = null; };
</script>

{#if errors.title}
  <Dialog title={errors.title} onclose={close}>
    <pre class="mono" style="white-space:pre-wrap;margin:0;user-select:text">{errors.message}</pre>
    {#snippet actions()}<button class="btn primary" onclick={close}>Close</button>{/snippet}
  </Dialog>
{/if}
