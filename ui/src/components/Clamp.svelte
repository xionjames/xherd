<script lang="ts">
  import { needsExpand } from '../lib/format';
  let { text, mono = false }: { text?: string; mono?: boolean } = $props();
  let open = $state(false);
  const more = $derived(needsExpand(text));
</script>

{#if text}
  <div class="clamp" class:mono class:open title={more && !open ? text : undefined}>{text}</div>
  {#if more}
    <button type="button" class="gtoggle" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Show less' : 'Show all'}</button>
  {/if}
{/if}
