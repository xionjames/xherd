<script lang="ts">
  // xterm.js bound to a terminal websocket (`/ws/term/:id`, `/ws/setup/:id`):
  // binary frames both ways, a JSON frame on resize.
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';

  let {
    path,
    title,
    height = 'min(70vh, 720px)',
    onclose,
    hint,
  }: { path: string; title: string; height?: string; onclose?: () => void; hint?: string } = $props();

  let host = $state<HTMLDivElement>();
  let conn = $state<'connecting' | 'open' | 'closed'>('connecting');
  let attempt = $state(0);

  $effect(() => {
    void attempt;
    const url = path;
    if (!host) return;
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
    const ws = new WebSocket(`${proto}://${location.host}/api/v1${url}?cols=${term.cols}&rows=${term.rows}`);
    ws.binaryType = 'arraybuffer';
    conn = 'connecting';
    const enc = new TextEncoder();
    ws.onopen = () => { conn = 'open'; term.focus(); };
    ws.onmessage = (e) => term.write(new Uint8Array(e.data as ArrayBuffer));
    ws.onclose = () => { conn = 'closed'; onclose?.(); };
    const input = term.onData((d) => ws.readyState === WebSocket.OPEN && ws.send(enc.encode(d)));
    const resized = term.onResize(({ cols, rows }) =>
      ws.readyState === WebSocket.OPEN && ws.send(JSON.stringify({ type: 'resize', cols, rows })));
    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host);
    return () => {
      ro.disconnect();
      input.dispose();
      resized.dispose();
      ws.onclose = null;
      ws.close();
      term.dispose();
    };
  });
</script>

<section class="term" aria-label="Terminal">
  <div class="tbar">
    <span class="twin on">{title}</span>
    <span class="grow"></span>
    {#if conn === 'closed'}<button class="tbtn" onclick={() => attempt++}>Reconnect</button>{/if}
  </div>
  <div class="screen" style:height bind:this={host}></div>
  <div class="tfoot">
    <span>{conn === 'open' ? '● attached' : conn === 'connecting' ? 'connecting…' : 'detached'}</span>
    {#if hint}<span>{hint}</span>{/if}
  </div>
</section>

<style>
  .screen { padding: 8px 4px 8px 10px; }
</style>
