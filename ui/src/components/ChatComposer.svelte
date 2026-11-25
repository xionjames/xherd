<script lang="ts">
  // Composer under the Chat view (#202 R10-R12): the text goes through the
  // `send` action; dropped and pasted files are stored and their paths added
  // to the message being written; `/` opens the pack's slash commands.
  import { post, uploadFiles } from '../lib/api';
  import { slashMatches } from '../lib/chat';
  import { pastePaths } from '../lib/upload';
  import { app, upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import type { Job } from '../lib/types';

  let { agent }: { agent: Job } = $props();
  // Keyed on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const blocked = $derived(agent.status === 'blocked');
  const answersQuestion = $derived(agent.gate?.kind === 'question');
  const commands = $derived(
    app.state?.plugins.find((p) => p.name === agent.runtime)?.slashCommands?.[agent.pack ?? ''] ?? [],
  );

  let text = $state('');
  let busy = $state(false);
  let progress = $state<number | null>(null);
  let dropping = $state(false);
  let picked = $state(0);
  let box = $state<HTMLTextAreaElement>();

  const matches = $derived(slashMatches(text, commands));
  $effect(() => { void matches.length; picked = 0; });
  $effect(() => { void id; text = ''; });

  function grow() {
    if (!box) return;
    box.style.height = 'auto';
    box.style.height = `${box.scrollHeight}px`;
  }
  async function send(e?: SubmitEvent) {
    e?.preventDefault();
    if (!text.trim() || busy || blocked) return;
    busy = true;
    try {
      upsertJob((await post<{ job: Job }>(`/jobs/${id}/send`, { text })).job);
      text = '';
      queueMicrotask(grow);
    } catch (err) { showError('Could not send the message', err); }
    finally { busy = false; }
  }
  function complete(cmd: string) {
    text = `${cmd} `;
    box?.focus();
    queueMicrotask(grow);
  }
  function key(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (matches.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        picked = (picked + (e.key === 'ArrowDown' ? 1 : matches.length - 1)) % matches.length;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && text !== matches[picked])) {
        e.preventDefault();
        complete(matches[picked]);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // One upload per drop or paste; the stored paths join the message (R11).
  async function upload(list: File[]) {
    if (!list.length || progress !== null || blocked) return;
    progress = 0;
    try {
      const paths = await uploadFiles(id, list, (f) => (progress = f));
      text = `${text}${text && !/\s$/.test(text) ? ' ' : ''}${pastePaths(paths)}`;
      queueMicrotask(grow);
      box?.focus();
    } catch (err) { showError('Could not upload', err); }
    finally { progress = null; }
  }
  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function onpaste(e: ClipboardEvent) {
    const list = [...(e.clipboardData?.files ?? [])];
    if (!list.length) return;
    e.preventDefault();
    void upload(list);
  }
  function ondragover(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dropping = true;
  }
  function ondrop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dropping = false;
    void upload([...(e.dataTransfer?.files ?? [])]);
  }
</script>

<form class="card composer" class:dropping onsubmit={send} {ondragover} {ondrop}
  ondragleave={() => (dropping = false)} aria-label="Message the agent">
  {#if matches.length}
    <ul class="menu" role="listbox" aria-label="Slash commands">
      {#each matches as cmd, i (cmd)}
        <li role="option" aria-selected={i === picked}>
          <button type="button" class:on={i === picked} tabindex="-1"
            onmousedown={(e) => { e.preventDefault(); complete(cmd); }}>{cmd}</button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="row">
    <textarea rows="1" bind:this={box} bind:value={text} oninput={grow} onkeydown={key} {onpaste}
      aria-label="Message" placeholder={blocked ? 'Blocked' : answersQuestion ? 'Answer the question' : 'Message the agent, or / for commands'}
      disabled={blocked || busy}></textarea>
    <button class="btn primary" type="submit" disabled={blocked || busy || !text.trim()}>Send</button>
  </div>
  <span class="hint">
    {#if blocked}
      The agent is blocked{agent.blockReason ? `: ${agent.blockReason}` : ''}. Resolve it before sending.
    {:else if progress !== null}
      Uploading {Math.round(progress * 100)}%…
    {:else if answersQuestion}
      The agent asked a question: this message answers it.
    {:else}
      Enter sends, Shift+Enter starts a new line. Drop or paste files to add their paths.
    {/if}
  </span>
</form>

<style>
  .composer { position: relative; display: flex; flex-direction: column; gap: 8px; padding: 12px 16px; }
  .composer.dropping { outline: 2px dashed var(--accent); outline-offset: -2px; }
  .composer .row { align-items: flex-end; flex-wrap: nowrap; }
  .composer textarea { flex: 1; resize: none; max-height: 40vh; overflow-y: auto; }
  .menu {
    position: absolute; left: 16px; right: 16px; bottom: 100%; margin: 0 0 4px; padding: 4px;
    list-style: none; max-height: 40vh; overflow-y: auto;
    background: var(--pen); border: 1px solid var(--fence); border-radius: 8px;
  }
  .menu button {
    display: block; width: 100%; padding: 6px 10px; text-align: left; border: 0; border-radius: 6px;
    background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .menu button.on { background: var(--accent-wash); }
</style>
