<script lang="ts">
  // Agent memory (R29): the review step may propose a new memory for the saved
  // agent this job came from. Accept writes it to the agent (version + 1; only
  // jobs created later get it), Dismiss drops it. Nothing is committed.
  import Panel from '../components/Panel.svelte';
  import Callout from '../components/Callout.svelte';
  import DiffLines from '../components/DiffLines.svelte';
  import Seg from '../components/Seg.svelte';
  import type { Job } from '../lib/types';
  import { get, post } from '../lib/api';
  import { upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { lineDiff } from '../lib/diff';
  import { agentLabel, jobAgent } from '../lib/agents';
  import { agoText } from '../lib/format';

  let { agent }: { agent: Job } = $props();
  type View = {
    proposal: { file: string; reason: string; at: number } | null;
    proposed?: string | null;
    agent?: { id: string; name: string } | null;
    current?: string | null;
    version?: number | null;
  };
  const id = $derived(agent.id);
  const label = $derived.by(() => {
    const a = jobAgent(agent);
    return a ? agentLabel(a.name, a.source) : undefined;
  });
  const at = $derived(agent.agentMemoryProposal?.at);
  let view = $state<View | null>(null);
  let mode = $state<'diff' | 'edit'>('diff');
  let text = $state('');
  let busy = $state(false);

  async function load(jobId: string) {
    try {
      view = await get<View>(`/jobs/${jobId}/agent-memory`);
      text = view.proposed ?? '';
      mode = 'diff';
    } catch (e) { showError('Could not read the agent memory proposal', e); }
  }
  $effect(() => { void at; load(id); });

  const edited = $derived(!!view?.proposed && text !== view.proposed);
  const diff = $derived(lineDiff(view?.current ?? '', text));
  const tooLong = $derived([...text].length > 4000);

  async function act(action: 'agent-memory-accept' | 'agent-memory-dismiss') {
    busy = true;
    try {
      const body = action === 'agent-memory-accept' && edited ? { text } : {};
      await post(`/jobs/${id}/${action}`, body);
      await load(id);
    } catch (e) {
      showError(action === 'agent-memory-accept' ? 'Could not save the agent memory' : 'Could not dismiss the proposal', e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="stack">
  <Panel title="Agent memory" hint="What every later job of this agent reads first (up to 4 000 characters).">
    {#if !view}
      <p class="muted">Loading…</p>
    {:else if !view.proposal}
      <p class="empty">No proposal. At review, the job may propose a new memory for {label ?? 'its agent'}.</p>
    {:else}
      <p><b style="font-weight:500">{view.proposal.reason}</b> <span class="hint">proposed {agoText(view.proposal.at)}</span></p>
      {#if view.proposed == null}
        <Callout tone="bad">{view.proposal.file} is gone from the worktree: dismiss this proposal.</Callout>
      {:else}
        <Seg label="Show" options={[{ value: 'diff', label: 'Diff' }, { value: 'edit', label: 'Edit' }]} value={mode} onchange={(v) => (mode = v)} />
        {#if mode === 'edit'}
          <textarea class="mono ed" bind:value={text} rows="14" aria-label="Proposed agent memory"></textarea>
          {#if edited}<p class="hint">Edited: your text replaces the job's. <button class="btn ghost sm" onclick={() => (text = view?.proposed ?? '')}>Undo edits</button></p>{/if}
          {#if tooLong}<Callout tone="bad">Over 4 000 characters: shorten it.</Callout>{/if}
        {:else}
          <p class="hint mono">{label ?? view.agent?.name ?? 'agent'} memory{view.version != null ? `, version ${view.version}` : ''}</p>
          <DiffLines lines={diff} />
        {/if}
      {/if}
    {/if}
    {#snippet foot()}
      {#if view?.proposal}
        <button class="btn ghost" disabled={busy} onclick={() => act('agent-memory-dismiss')}>Dismiss</button>
        <button class="btn primary" disabled={busy || view.proposed == null || !text.trim() || tooLong} onclick={() => act('agent-memory-accept')}>
          Save to the agent</button>
      {/if}
    {/snippet}
  </Panel>
</div>

<style>
  .ed { width: 100%; min-height: 14em; }
</style>
