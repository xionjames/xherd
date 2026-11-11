<script lang="ts">
  // The header of every page but the agent's: title, optional status next to
  // it (`meta`), actions on the right, a line under it (`sub`) and tabs.
  import type { Snippet } from 'svelte';
  let { title, sub, meta, actions, tabs }: {
    title: string;
    sub?: string | Snippet;
    meta?: Snippet;
    actions?: Snippet;
    tabs?: Snippet;
  } = $props();
</script>

<div class="phead" class:notabs={!tabs}>
  <div class="ph">
    <h1>{title}</h1>
    {@render meta?.()}
    {#if actions}<div class="pa">{@render actions()}</div>{/if}
  </div>
  {#if typeof sub === 'string'}<p class="sub">{sub}</p>{:else if sub}<p class="sub">{@render sub()}</p>{/if}
  {@render tabs?.()}
</div>
