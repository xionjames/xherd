<script lang="ts">
  // A Herd view plugin (R9, R12): its page in a sandboxed frame with an
  // opaque origin, fed protocol v1 `data` messages (`lib/viewPlugin.ts`).
  // The frame fills the viewport below its top; the view scrolls inside.
  // Only `ready` and `open` of a known agent are taken back.
  import { onMount } from 'svelte';
  import { acceptMessage, dataMessage } from '../lib/viewPlugin';
  import { isDark } from '../lib/theme.svelte';
  import type { HerdViewProps } from '../lib/herdViews';

  let { agents, flows, open, plugin = '', entry = '', label = '' }: HerdViewProps = $props();

  let frame = $state<HTMLIFrameElement>();
  let ready = $state(false);
  let height = $state(480);

  const src = $derived(
    `/api/v1/plugins/${encodeURIComponent(plugin)}/view/${entry.split('/').map(encodeURIComponent).join('/')}`,
  );
  const ids = $derived(new Set(agents.map((a) => a.id)));
  const byId = $derived(new Map(agents.map((a) => [a.id, a])));

  // A new page in the frame must say `ready` again before it gets data.
  $effect(() => {
    void src;
    ready = false;
  });

  // Send the data when the view is ready and whenever agents, flows or the
  // theme change. Only to the frame's window; its origin is opaque, so `*`.
  $effect(() => {
    const msg = dataMessage(agents, flows, isDark() ? 'dark' : 'light');
    if (ready) frame?.contentWindow?.postMessage(msg, '*');
  });

  function onmessage(e: MessageEvent) {
    const req = acceptMessage(e, frame?.contentWindow, ids);
    if (!req) return;
    if (req.kind === 'ready') ready = true;
    else {
      const a = byId.get(req.id);
      if (a) open(a);
    }
  }

  function measure() {
    if (!frame) return;
    const top = frame.getBoundingClientRect().top + window.scrollY;
    height = Math.max(240, window.innerHeight - top - 16);
  }

  onMount(() => {
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  });
</script>

<svelte:window {onmessage} />

<div class="wrap">
  <iframe bind:this={frame} {src} title={label || plugin} sandbox="allow-scripts" referrerpolicy="no-referrer"
    style:height="{height}px"></iframe>
</div>

<style>
  .wrap { padding: 16px var(--gut) 0; }
  iframe { display: block; width: 100%; border: 0; background: transparent; }
</style>
