<script lang="ts">
  // Overview (spec §17.2): a frame around the chosen Herd view (`?layout=`,
  // `lib/herdViews.ts`). Each view draws the whole body; the page keeps the
  // title, the Layout switch and the kill switch.
  import PageHead from '../components/PageHead.svelte';
  import Seg from '../components/Seg.svelte';
  import Dialog from '../components/Dialog.svelte';
  import { app } from '../lib/store.svelte';
  import { route, go } from '../lib/router.svelte';
  import { HERD_VIEWS, pickView } from '../lib/herdViews';
  import { post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import type { Agent } from '../lib/types';

  const agents = $derived(app.state?.agents ?? []);
  const flows = $derived(app.state?.flows ?? []);
  const view = $derived(pickView(route.query.get('layout')));
  // The first view is the default and leaves `layout` out; the filter stays.
  const setLayout = (id: string) =>
    go('/', { f: route.query.get('f') ?? undefined, layout: id === HERD_VIEWS[0].id ? undefined : id });
  const open = (a: Agent) => go(`/agent/${a.name}`);

  // The kill switch (R-032): no handoff notes, nothing new starts.
  const pausable = $derived(agents.filter((a) => ['queued', 'running', 'idle', 'blocked', 'waiting-input'].includes(a.status)).length);
  let confirmPause = $state(false);
  async function pauseAll() {
    confirmPause = false;
    try {
      await post('/agents/pause-all');
    } catch (e) {
      showError('Could not pause the agents', e);
    }
  }
</script>

<PageHead title="Herd">
  {#snippet actions()}
    <Seg label="Layout" value={view.id} onchange={setLayout}
      options={HERD_VIEWS.map((v) => ({ value: v.id, label: v.label }))} />
    {#if pausable}<button class="btn" onclick={() => (confirmPause = true)} title="Kill switch: pause every running and queued agent now">Pause all</button>{/if}
  {/snippet}
</PageHead>

<view.component {agents} {flows} {open} />

{#if confirmPause}
  <Dialog title="Pause all {pausable} agents?" onclose={() => (confirmPause = false)}>
    <p>Their sessions end now, without a handoff note, and nothing queued starts. Resume each one when you want it back.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmPause = false)}>Keep them running</button>
      <button class="btn danger" onclick={pauseAll}>Pause all</button>
    {/snippet}
  </Dialog>
{/if}
