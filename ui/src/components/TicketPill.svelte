<script lang="ts">
  // An agent's ticket (#114): its short id as a pill, a link when it has one.
  // `link={false}` inside another link (a card).
  import type { Ticket } from '../lib/types';
  import { ticketPill } from '../lib/links';
  let { ticket, link = true }: { ticket?: Ticket; link?: boolean } = $props();
  const p = $derived(ticketPill(ticket));
</script>

{#if p}
  {#if p.href && link}<a class="pill acc mono" href={p.href} target="_blank" rel="noopener noreferrer" title="Open the ticket" onclick={(e) => e.stopPropagation()}>{p.label}</a>
  {:else}<span class="pill acc mono">{p.label}</span>{/if}
{/if}
