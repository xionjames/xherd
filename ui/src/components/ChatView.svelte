<script lang="ts">
  // The job's session as a conversation (#202 R8, R9): the chat log from
  // `/ws/chat/:id`, folded into one item per tool, the latest plan pinned.
  import { render } from '../lib/markdown';
  import { downloadUrl } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { fold, linkPaths, resolutionText, runningTool } from '../lib/chat';
  import type { ChatEvent, Job } from '../lib/types';

  let { agent }: { agent: Job } = $props();
  // Keyed on the id and workdir: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const name = $derived(agent.name);
  const workdir = $derived(agent.workdir);
  const working = $derived(agent.status === 'running');

  let events = $state<ChatEvent[]>([]);
  let conn = $state<'connecting' | 'open' | 'closed'>('connecting');
  let replayed = $state(false);
  let unknown = $state(0);
  let open = $state<Record<string, boolean>>({});
  let more = $state<Record<string, boolean>>({});
  let list = $state<HTMLDivElement>();

  const folded = $derived(fold(events));
  const tool = $derived(runningTool(folded.items));
  const EDITS = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit'];
  const CUT = 600;

  // Connects again from the last `seq` seen; a new job id starts from scratch.
  $effect(() => {
    const job = id;
    events = [];
    replayed = false;
    unknown = 0;
    let last = 0;
    let ws: WebSocket | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let gone = false;
    const connect = () => {
      conn = 'connecting';
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      ws = new WebSocket(`${proto}://${location.host}/api/v1/ws/chat/${job}?after=${last}`);
      ws.onopen = () => (conn = 'open');
      ws.onmessage = (m) => {
        let v: { type: string; event?: ChatEvent; unknown?: number; count?: number };
        try { v = JSON.parse(m.data as string); } catch { return; }
        if (v.type === 'hello') unknown = v.unknown ?? 0;
        else if (v.type === 'unknown') unknown = v.count ?? unknown;
        else if (v.type === 'replayed') replayed = true;
        else if (v.type === 'event' && v.event) {
          last = Math.max(last, v.event.seq);
          events.push(v.event);
        }
      };
      ws.onclose = () => {
        if (gone) return;
        conn = 'closed';
        timer = setTimeout(connect, 2000);
      };
    };
    connect();
    return () => {
      gone = true;
      clearTimeout(timer);
      ws?.close();
    };
  });

  // Stay at the bottom while new lines arrive, unless the reader scrolled up.
  let stick = true;
  function scrolled() {
    if (list) stick = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
  }
  $effect(() => {
    void folded.items.length;
    void working;
    if (stick && list) queueMicrotask(() => list && (list.scrollTop = list.scrollHeight));
  });

  const html = (text: string) => {
    const h = render(text, undefined, true).html;
    return workdir ? linkPaths(h, workdir, (p) => downloadUrl(id, p)) : h;
  };
</script>

<section class="chat card" aria-label="Chat">
  {#if folded.plan}
    <div class="plan" aria-label="Plan">
      <strong>Plan</strong>
      <ul>
        {#each folded.plan as p, i (i)}
          <li class={p.status}>
            <span class="mk" aria-hidden="true">{p.status === 'completed' ? '✓' : p.status === 'in-progress' ? '▸' : '○'}</span>
            <span>{p.text}</span><span class="sr"> ({p.status})</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
  <div class="list" bind:this={list} onscroll={scrolled} role="log" aria-live="polite">
    {#if !replayed && conn !== 'closed'}
      <p class="hint">Loading the conversation…</p>
    {:else if !folded.items.length}
      <p class="hint">Nothing in the conversation yet.</p>
    {/if}
    {#each folded.items as e (e.kind === 'tool' || e.kind === 'permission' || e.kind === 'question' ? `${e.kind}:${e.id}` : e.seq)}
      {#if e.kind === 'user'}
        <div class="msg user" class:xherd={e.xherd}>
          <span class="who">{e.xherd ? 'xherd' : 'You'}</span>
          <div class="body">{e.text}</div>
        </div>
      {:else if e.kind === 'assistant'}
        <div class="msg assistant">
          <span class="who">Agent</span>
          <div class="body md">{@html html(e.text)}</div>
        </div>
      {:else if e.kind === 'tool'}
        {@const key = `t${e.id}`}
        <div class="tool {e.status}">
          <button class="head" aria-expanded={!!open[key]} disabled={!e.result} onclick={() => (open[key] = !open[key])}>
            <span class="mk" aria-hidden="true">{e.status === 'running' ? '…' : e.status === 'error' ? '✗' : '✓'}</span>
            <span class="nm">{e.name}</span>
            <span class="sm">{e.summary}</span>
            <span class="sr">{e.status}</span>
          </button>
          {#if EDITS.includes(e.name)}
            <a class="hint" href={href(`/jobs/${name}`, { s: 'changes' })}>Changes</a>
          {/if}
          {#if open[key] && e.result}
            <pre class="res">{more[key] || e.result.length <= CUT ? e.result : `${e.result.slice(0, CUT)}…`}</pre>
            {#if e.result.length > CUT && !more[key]}
              <button class="btn sm" onclick={() => (more[key] = true)}>Show more</button>
            {/if}
          {/if}
        </div>
      {:else if e.kind === 'notice'}
        <div class="notice">{e.text}</div>
      {:else if e.kind === 'permission' || e.kind === 'question'}
        <!-- Answerable cards come with T24; a resolved one shows how it ended. -->
        <div class="notice">
          {e.kind === 'permission' ? `${e.tool}: ${e.summary}` : (e.questions[0]?.question ?? 'Question')}
          {#if resolutionText(e)} · {resolutionText(e)}{:else} · waiting for an answer{/if}
        </div>
      {/if}
    {/each}
    {#if working}
      <div class="working" aria-live="polite">Working{tool ? `: ${tool}` : '…'}</div>
    {/if}
  </div>
  {#if conn === 'closed'}
    <p class="hint conn">Connection lost, reconnecting…</p>
  {/if}
  {#if unknown}
    <p class="hint conn">{unknown} line{unknown === 1 ? '' : 's'} of the transcript could not be shown: see the terminal.</p>
  {/if}
</section>

<style>
  .chat { display: flex; flex-direction: column; min-height: 0; }
  .list { display: flex; flex-direction: column; gap: 12px; padding: 16px; max-height: min(70vh, 720px); overflow-y: auto; }
  .plan { padding: 12px 16px; border-bottom: 1px solid var(--fence); background: var(--well); }
  .plan ul { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .plan li { display: flex; gap: 8px; }
  .plan li.completed { color: var(--ink-3); }
  .plan li.in-progress { color: var(--ink); font-weight: 600; }
  .mk { width: 1.2em; text-align: center; flex: none; }
  .msg { display: flex; flex-direction: column; gap: 2px; max-width: 80ch; }
  .msg.user { align-self: flex-end; align-items: flex-end; }
  .who { font-size: 12px; color: var(--ink-3); }
  .user .body { background: var(--accent-wash); border-radius: var(--r-card); padding: 8px 12px; white-space: pre-wrap; overflow-wrap: anywhere; }
  .user.xherd .body { background: var(--pen-2); font-family: var(--mono); font-size: 12.5px; }
  .assistant .body { overflow-wrap: anywhere; }
  .tool { border: 1px solid var(--fence); border-radius: var(--r-ctl); background: var(--well); padding: 2px 8px; }
  .tool.error { border-color: var(--halt); }
  .tool .head { all: unset; display: flex; gap: 8px; align-items: baseline; width: 100%; cursor: pointer; padding: 4px 0; font: 12.5px/1.5 var(--mono); }
  .tool .head:disabled { cursor: default; }
  .tool .head:focus-visible { outline: 2px solid var(--focus); }
  .tool .nm { font-weight: 600; flex: none; }
  .tool .sm { color: var(--ink-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .res { margin: 4px 0; padding: 8px; background: var(--term); color: var(--term-ink); border-radius: var(--r-ctl); font: 12px/1.5 var(--mono); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 40vh; overflow-y: auto; }
  .notice { align-self: center; color: var(--ink-3); font-size: 12.5px; }
  .working { color: var(--ink-3); font-size: 12.5px; }
  .conn { padding: 0 16px 12px; margin: 0; }
</style>
