<script lang="ts">
  // `terminal` panel: the agent's tmux session through /ws/term/:id.
  import TermView from '../components/TermView.svelte';
  import Callout from '../components/Callout.svelte';
  import ChatView from '../components/ChatView.svelte';
  import ChatComposer from '../components/ChatComposer.svelte';
  import WaitingCard from '../components/WaitingCard.svelte';
  import type { Job, JobView } from '../lib/types';
  import { post } from '../lib/api';
  import { app, upsertJob } from '../lib/store.svelte';
  import { cardFromWaiting } from '../lib/chat';
  import { chooseView, viewOf } from '../lib/jobView.svelte';
  import { showError } from '../lib/errors.svelte';

  let { agent }: { agent: Job } = $props();
  // Keyed on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  // Drop, paste and path links need a worktree to store into and read from.
  const files = $derived(agent.workdir ? { jobId: id, workdir: agent.workdir } : undefined);
  const live = $derived(['running', 'waiting-input', 'blocked', 'idle'].includes(agent.status));
  // Ask (R5): an agent without a session that can be resumed takes a question.
  const askable = $derived(['paused', 'parked', 'watching', 'error'].includes(agent.status) && !agent.worktreeRemovedAt);

  // Chat / Terminal (#202 R7): only for a runtime with a chat view. Both show
  // the same session; the choice is kept per job in this browser.
  const hasChat = $derived(!!app.state?.plugins.find((p) => p.name === agent.runtime)?.chat);
  const configView = $derived((app.state?.config.ui as { jobView?: unknown } | undefined)?.jobView);
  const view = $derived(viewOf(id, hasChat, configView));
  const chat = $derived(hasChat && view === 'chat');

  let text = $state('');
  let busy = $state(false);
  let box = $state<HTMLTextAreaElement>();
  function grow() {
    if (!box) return;
    box.style.height = 'auto';
    box.style.height = `${box.scrollHeight}px`;
  }
  async function send(action: 'ask' | 'finish', body: object, label: string) {
    busy = true;
    try {
      upsertJob((await post<{ job: Job }>(`/jobs/${id}/${action}`, body)).job);
      if (action === 'ask') { text = ''; queueMicrotask(grow); }
    } catch (e) { showError(label, e); }
    finally { busy = false; }
  }
  function ask(e: SubmitEvent) {
    e.preventDefault();
    if (text.trim() && !busy) send('ask', { text }, 'Could not ask the agent');
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      (e.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
    }
  }
</script>

<div class="stack">
  {#if hasChat}
    <div class="row">
      <button class="btn ghost sm" onclick={() => chooseView(id, chat ? 'terminal' : 'chat')}>
        {chat ? 'Open the terminal' : 'Open the chat'}
      </button>
    </div>
  {/if}
  <!-- The hook holds the request, so Claude Code shows no prompt in the terminal: the cards are here too. -->
  {#if !chat}
    {#each agent.waiting ?? [] as w (w.id)}<WaitingCard jobId={id} card={cardFromWaiting(w)} from="terminal" />{/each}
  {/if}
  {#if chat}<ChatView {agent} />{/if}
  {#if chat && live}<ChatComposer {agent} />{/if}
  {#if agent.askHold}
    <Callout tone="info">
      <div class="row" style="justify-content:space-between">
        <span>Asked by you: the session stays open until you finish.</span>
        <button class="btn sm" disabled={busy} onclick={() => send('finish', {}, 'Could not finish')}>Finish</button>
      </div>
    </Callout>
  {/if}
  {#if live}
    {#if !chat}<TermView path={`/ws/term/${id}`} title={agent.name} {files}
      hint={`tmux session ${agent.name}; leaving this page detaches, the agent keeps running.`} />{/if}
  {:else if askable}
    <form class="card ask" onsubmit={ask}>
      <label for="ask-text">No session: the agent is {agent.status}. Ask it something; it continues its last conversation and stays until you finish.</label>
      <div class="row">
        <textarea id="ask-text" rows="1" bind:this={box} bind:value={text} oninput={grow} onkeydown={key}
          placeholder="Why did the check fail?" disabled={busy}></textarea>
        <button class="btn primary" type="submit" disabled={busy || !text.trim()}>Ask</button>
      </div>
      <span class="hint">Enter sends, Shift+Enter starts a new line.</span>
    </form>
  {:else if !chat}
    <section class="term" aria-label="Terminal">
      <div class="scr c-dim">No session: the agent is {agent.status}.{agent.status === 'queued' ? ' It starts when a slot is free.' : ''}</div>
    </section>
  {/if}
</div>

<style>
  .ask { display: flex; flex-direction: column; gap: 8px; padding: 16px; }
  .ask .row { align-items: flex-end; flex-wrap: nowrap; }
  .ask textarea { flex: 1; resize: none; max-height: 40vh; overflow-y: auto; }
</style>
