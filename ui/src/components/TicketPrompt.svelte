<script lang="ts">
  // Once an agent's pull request is open, offer to post a summary on its ticket
  // (R-308): a prompt on the agent page that opens the Ticket tab with the
  // draft ready. It shows only when an enabled tracker reads the ticket and
  // nothing was posted yet; "Not now" is remembered in this browser. Nothing
  // is posted from here.
  import { get } from '../lib/api';
  import { href, route } from '../lib/router.svelte';
  import Callout from './Callout.svelte';
  import type { Agent } from '../lib/types';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const name = $derived(agent.name);
  const open = $derived(!!agent.prUrl);

  const key = $derived(`xherd.ticketPrompt.${id}`);
  const dismissed = () => { try { return localStorage.getItem(key) === '1'; } catch { return false; } };
  let hidden = $state(false);
  let status = $state<{ tracker: string | null; issue?: string; posted: boolean } | null>(null);

  const onTicketTab = $derived(route.query.get('s') === 'ticket');
  // Asked again when the tab changes, so a summary posted meanwhile ends the prompt.
  $effect(() => {
    void id; void route.query.get('s');
    hidden = dismissed();
    status = null;
    if (!open) return;
    get<{ tracker: string | null; issue?: string; posted: boolean }>(`/agents/${id}/ticket-status`)
      .then((s) => (status = s))
      .catch(() => (status = null));
  });
  function later() {
    hidden = true;
    try { localStorage.setItem(key, '1'); } catch { /* the prompt just comes back */ }
  }
</script>

{#if open && !hidden && !onTicketTab && status?.tracker && !status.posted}
  <Callout tone="info">
    The pull request is open. Post a short summary of the work to the ticket ({status.tracker} #{status.issue})?
    You read and edit it first; nothing is posted until you press Post.
    <a class="btn sm" href={href(`/agent/${name}`, { s: 'ticket', draft: '1' })}>Draft a summary</a>
    <button class="btn ghost sm" onclick={later}>Not now</button>
  </Callout>
{/if}
