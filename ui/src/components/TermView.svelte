<script lang="ts">
  // xterm.js bound to a terminal websocket (`/ws/term/:id`, `/ws/setup/:id`):
  // binary frames both ways, a JSON frame on resize.
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
  import { urlLinks } from '../lib/termLinks';

  // Links (typed URLs and OSC 8 hyperlinks) open in a new tab, http(s) only.
  const open = (_e: MouseEvent, uri: string) => {
    if (/^https?:\/\//i.test(uri)) window.open(uri, '_blank', 'noopener,noreferrer');
  };

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
    // The terminal's colours are the --term tokens of app.css.
    const css = getComputedStyle(host);
    const tok = (n: string) => css.getPropertyValue(n).trim();
    const term = new Terminal({
      fontFamily: tok('--mono'),
      fontSize: 12.5,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true, // the Unicode 11 addon needs it
      macOptionClickForcesSelection: true, // Option+drag selects even when the app grabs the mouse
      rightClickSelectsWord: true,
      linkHandler: { activate: open },
      theme: { background: tok('--term'), foreground: tok('--term-ink'), cursor: tok('--term-ink'), selectionBackground: tok('--term-sel') },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const linked = term.registerLinkProvider(urlLinks(term, open));
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11'; // widths of newer glyphs (the runtime's logo)
    term.open(host);
    // The WebGL renderer draws block and box characters itself (the DOM one
    // leaves them to the font); without WebGL the DOM renderer stays.
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch { /* no WebGL */ }
    fit.fit();
    // Copy on select: programs that grab the mouse (Claude Code) need Shift+drag
    // (Option+drag on a Mac) to select; the selection then lands on the clipboard.
    const picked = term.onSelectionChange(() => {
      const t = term.getSelection();
      if (t) navigator.clipboard?.writeText(t).catch(() => {});
    });
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
      picked.dispose();
      linked.dispose();
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
    <span>Select text with Shift+drag (Option+drag on a Mac); it is copied.</span>
  </div>
</section>

<style>
  .screen { padding: 8px 4px 8px 10px; }
</style>
