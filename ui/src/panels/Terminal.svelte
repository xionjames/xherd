<script lang="ts">
  // `terminal` panel: the agent's tmux session through /ws/term/:id.
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import type { Agent } from '../lib/types';

  let { agent }: { agent: Agent } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  let host = $state<HTMLDivElement>();
  let conn = $state<'connecting' | 'open' | 'closed'>('connecting');
  let attempt = $state(0);

  const live = $derived(['running', 'waiting-input', 'blocked', 'idle'].includes(agent.status));

  $effect(() => {
    void attempt;
    if (!live || !host) return;
    const term = new Terminal({
      fontFamily: "'JetBrains Mono', ui-monospace, Menlo, monospace",
      fontSize: 12.5,
      cursorBlink: true,
      scrollback: 5000,
      theme: { background: '#0c0a1f', foreground: '#d6d2f2', cursor: '#d6d2f2', selectionBackground: '#3a3370' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/api/v1/ws/term/${id}?cols=${term.cols}&rows=${term.rows}`);
    ws.binaryType = 'arraybuffer';
    conn = 'connecting';
    const enc = new TextEncoder();
    ws.onopen = () => { conn = 'open'; term.focus(); };
    ws.onmessage = (e) => term.write(new Uint8Array(e.data as ArrayBuffer));
    ws.onclose = () => { conn = 'closed'; };
    const input = term.onData((d) => ws.readyState === WebSocket.OPEN && ws.send(enc.encode(d)));
    const resized = term.onResize(({ cols, rows }) =>
      ws.readyState === WebSocket.OPEN && ws.send(JSON.stringify({ type: 'resize', cols, rows })));
    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host);
    return () => {
      ro.disconnect();
      input.dispose();
      resized.dispose();
      ws.close();
      term.dispose();
    };
  });
</script>

<div class="stack">
  <section class="term" aria-label="Terminal">
    <div class="tbar">
      <span class="twin on">{agent.name}</span>
      <span class="grow"></span>
      {#if conn === 'closed' && live}
        <button class="tbtn" onclick={() => attempt++}>Reconnect</button>
      {/if}
    </div>
    {#if live}
      <div class="screen" bind:this={host}></div>
    {:else}
      <div class="scr c-dim">No session: the agent is {agent.status}. Resume it to get a terminal.</div>
    {/if}
    <div class="tfoot">
      <span>{conn === 'open' ? '● attached' : conn === 'connecting' ? 'connecting…' : 'detached'}</span>
      <span>tmux session <span class="mono">{agent.name}</span>; leaving this page detaches, the agent keeps running.</span>
    </div>
  </section>
</div>

<style>
  .screen { height: min(70vh, 720px); padding: 8px 4px 8px 10px; }
</style>
