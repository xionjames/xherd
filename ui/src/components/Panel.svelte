<script lang="ts">
  // The one panel: a card with a header (title, hint, then `head` for actions
  // or controls), a body, and a footer for its actions (primary last).
  // `flush` leaves the body unpadded, for a table or a `.list`.
  import type { Snippet } from 'svelte';
  let { title, hint, tone, flush = false, label, head, foot, children }: {
    title?: string;
    hint?: string;
    tone?: 'need' | 'err';
    flush?: boolean;
    label?: string;
    head?: Snippet;
    foot?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<section class="card {tone ?? ''}" aria-label={label ?? title}>
  {#if title || head}
    <div class="ch">
      {#if title}<div class="t"><h2>{title}</h2>{#if hint}<p class="hint">{hint}</p>{/if}</div>{/if}
      {@render head?.()}
    </div>
  {/if}
  {#if children}
    {#if flush}{@render children()}{:else}<div class="cb">{@render children()}</div>{/if}
  {/if}
  {#if foot}<div class="cf">{@render foot()}</div>{/if}
</section>
