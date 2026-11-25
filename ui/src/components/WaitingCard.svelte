<script lang="ts">
  // A permission request or a question the agent waits on (#202 R23): one
  // button per option while it waits, how it ended afterwards. Used in the
  // chat (`chat`), the Inbox (`inbox`) and above the terminal (`terminal`).
  import { post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { cardButtons, cardNote, type Card } from '../lib/chat';
  import type { AnsweredFrom } from '../lib/types';

  let { jobId, card, from }: { jobId: string; card: Card; from: AnsweredFrom } = $props();
  let busy = $state(false);
  const buttons = $derived(cardButtons(card));
  const note = $derived(cardNote(card));

  async function answer(value: string | number) {
    busy = true;
    try {
      await post(
        card.kind === 'permission' ? `/jobs/${jobId}/permissions/${card.id}` : `/jobs/${jobId}/questions/${card.id}`,
        card.kind === 'permission' ? { answer: value, from } : { option: value, from },
      );
    } catch (e) { showError('Could not send the answer', e); }
    finally { busy = false; }
  }
</script>

<div class="wcard" class:waiting={card.res.state === 'waiting'}>
  <div class="what">
    <b>{card.kind === 'permission' ? `Permission: ${card.title}` : card.title}</b>
    <span class="txt">{card.text}</span>
    {#if card.kind === 'question'}
      {#each card.questions[0]?.options ?? [] as o, i (i)}
        {#if o.description}<span class="hint">{o.label}: {o.description}</span>{/if}
      {/each}
    {/if}
  </div>
  {#if buttons.length}
    <div class="row">
      {#each buttons as b (b.label)}
        <button class="btn sm" disabled={busy} onclick={() => answer(b.value)}>{b.label}</button>
      {/each}
    </div>
  {/if}
  {#if note}<span class="hint">{note}</span>{/if}
</div>

<style>
  .wcard { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border: 1px solid var(--fence); border-radius: var(--r-ctl); background: var(--well); }
  .wcard.waiting { border-color: var(--gate); }
  .what { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .txt { font-family: var(--mono); font-size: 12.5px; overflow-wrap: anywhere; white-space: pre-wrap; }
</style>
