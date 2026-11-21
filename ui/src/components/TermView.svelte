<script lang="ts">
  // xterm.js bound to a terminal websocket (`/ws/term/:id`, `/ws/setup/:id`):
  // binary frames both ways, a JSON frame on resize.
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { Unicode11Addon } from '@xterm/addon-unicode11';
  import { pathLinks, urlLinks } from '../lib/termLinks';
  import { pastePaths } from '../lib/upload';
  import { downloadUrl, uploadFiles } from '../lib/api';
  import { showError } from '../lib/errors.svelte';

  // Links (typed URLs and OSC 8 hyperlinks) open in a new tab, http(s) only.
  const open = (_e: MouseEvent, uri: string) => {
    if (/^https?:\/\//i.test(uri)) window.open(uri, '_blank', 'noopener,noreferrer');
  };

  // `files` (set only for an agent's terminal with a worktree) turns on file
  // drop and paste (R7–R9) and links on paths under the workdir (R10).
  let {
    path,
    title,
    height = 'min(70vh, 720px)',
    onclose,
    hint,
    files,
  }: {
    path: string;
    title: string;
    height?: string;
    onclose?: () => void;
    hint?: string;
    files?: { jobId: string; workdir: string };
  } = $props();

  let host = $state<HTMLDivElement>();
  let conn = $state<'connecting' | 'open' | 'closed'>('connecting');
  let attempt = $state(0);
  let dropping = $state(false);
  let progress = $state<number | null>(null); // 0..1 while an upload runs
  // The effect below must not read `files` itself: the caller passes a new
  // object on every agent update. These strings stay equal, so it re-runs
  // only when the job or its workdir changes.
  const target = $derived(files?.jobId);
  const workdir = $derived(files?.workdir);

  $effect(() => {
    void attempt;
    const url = path;
    if (!host) return;
    const screen = host;
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
    const pathLinked = target && workdir
      ? term.registerLinkProvider(pathLinks(term, workdir, (_e, p) => {
          const a = document.createElement('a');
          a.href = downloadUrl(target, p);
          a.download = '';
          a.click();
        }))
      : undefined;
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
    const ws = new WebSocket(`${proto}://${location.host}/api/v1${url}${url.includes('?') ? '&' : '?'}cols=${term.cols}&rows=${term.rows}`);
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

    // One upload per drop or paste; the stored paths are pasted, never run.
    const upload = async (list: File[]) => {
      if (!target || !list.length || progress !== null) return;
      progress = 0;
      try {
        const paths = await uploadFiles(target, list, (f) => (progress = f));
        term.paste(pastePaths(paths));
        term.focus();
      } catch (e) {
        showError('Could not upload', e);
      } finally {
        progress = null;
      }
    };
    const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
    const onDragOver = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      dropping = true;
    };
    const onDragLeave = (e: DragEvent) => {
      if (!screen.contains(e.relatedTarget as Node | null)) dropping = false;
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      dropping = false;
      void upload([...(e.dataTransfer?.files ?? [])]);
    };
    // Capture phase: xterm's own paste handler must not see a file paste.
    // Text pastes carry no files and go through untouched.
    const onPaste = (e: ClipboardEvent) => {
      const list = [...(e.clipboardData?.files ?? [])];
      if (!list.length) return;
      e.preventDefault();
      e.stopPropagation();
      void upload(list);
    };
    if (target) {
      screen.addEventListener('dragover', onDragOver);
      screen.addEventListener('dragleave', onDragLeave);
      screen.addEventListener('drop', onDrop);
      screen.addEventListener('paste', onPaste, true);
    }
    return () => {
      screen.removeEventListener('dragover', onDragOver);
      screen.removeEventListener('dragleave', onDragLeave);
      screen.removeEventListener('drop', onDrop);
      screen.removeEventListener('paste', onPaste, true);
      dropping = false;
      pathLinked?.dispose();
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
    {#if progress !== null}<span class="tbtn chip" role="status">Uploading… {Math.round(progress * 100)}%</span>{/if}
    {#if conn === 'closed'}<button class="tbtn" onclick={() => attempt++}>Reconnect</button>{/if}
  </div>
  <div class="screen" class:dropping style:height bind:this={host}></div>
  <div class="tfoot">
    <span>{conn === 'open' ? '● attached' : conn === 'connecting' ? 'connecting…' : 'detached'}</span>
    {#if hint}<span>{hint}</span>{/if}
    <span>Select text with Shift+drag (Option+drag on a Mac); it is copied.</span>
    {#if files}<span>Drop or paste files to send them to the agent.</span>{/if}
  </div>
</section>

<style>
  .screen { padding: 8px 4px 8px 10px; }
  .screen.dropping { outline: 2px dashed var(--accent); outline-offset: -4px; }
  .chip { display: inline-flex; align-items: center; cursor: default; }
</style>
