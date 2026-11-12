<script lang="ts">
  // Shared memory (spec §16.6, #65): the review step may propose an update of
  // the repository's AGENTS.md. Accept commits it (and a CLAUDE.md importing
  // it) on the agent's branch, so it ships with the PR; Dismiss drops it.
  import Panel from '../components/Panel.svelte';
  import Callout from '../components/Callout.svelte';
  import DiffLines from '../components/DiffLines.svelte';
  import Seg from '../components/Seg.svelte';
  import type { Agent } from '../lib/types';
  import { get, post } from '../lib/api';
  import { upsertAgent } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { lineDiff } from '../lib/diff';
  import { agoText } from '../lib/format';

  let { agent }: { agent: Agent } = $props();
  type View = {
    proposal: { file: string; reason: string; at: number } | null;
    proposed?: string | null;
    current?: string | null;
    claude?: { current: string | null; next: string | null };
  };
  const id = $derived(agent.id);
  const at = $derived(agent.memoryProposal?.at);
  let view = $state<View | null>(null);
  let mode = $state<'diff' | 'edit'>('diff');
  let text = $state('');
  let busy = $state(false);

  async function load(agentId: string) {
    try {
      view = await get<View>(`/agents/${agentId}/memory`);
      text = view.proposed ?? '';
      mode = 'diff';
    } catch (e) { showError('Could not read the memory proposal', e); }
  }
  // Reload when the agent or its proposal changes, not on every update.
  $effect(() => { void at; load(id); });

  const edited = $derived(!!view?.proposed && text !== view.proposed);
  const agents = $derived(lineDiff(view?.current ?? '', text));
  const claude = $derived(view?.claude?.next ? lineDiff(view.claude.current ?? '', view.claude.next) : null);
  const blocked = $derived(
    agent.status === 'running' ? 'The agent is working: accept when it stops.'
    : agent.prUrl ? 'The pull request is open and xherd never pushes: copy the text into AGENTS.md yourself, or dismiss it.'
    : '',
  );

  async function act(action: 'memory-accept' | 'memory-dismiss') {
    busy = true;
    try {
      const body = action === 'memory-accept' && edited ? { text } : {};
      upsertAgent((await post<{ agent: Agent }>(`/agents/${id}/${action}`, body)).agent);
      await load(id);
    } catch (e) {
      showError(action === 'memory-accept' ? 'Could not commit AGENTS.md' : 'Could not dismiss the proposal', e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="stack">
  <Panel title="Shared memory" hint="AGENTS.md: what every future agent on this repository reads first.">
    {#if !view}
      <p class="muted">Loading…</p>
    {:else if !view.proposal}
      <p class="empty">No proposal. At review, the agent proposes what future agents here should know (commands, conventions, traps) as an update of AGENTS.md.</p>
    {:else}
      <p><b style="font-weight:500">{view.proposal.reason}</b> <span class="hint">proposed {agoText(view.proposal.at)}</span></p>
      {#if view.proposed == null}
        <Callout tone="bad">{view.proposal.file} is gone from the worktree: dismiss this proposal.</Callout>
      {:else}
        <Seg label="Show" options={[{ value: 'diff', label: 'Diff' }, { value: 'edit', label: 'Edit' }]} value={mode} onchange={(v) => (mode = v)} />
        {#if mode === 'edit'}
          <textarea class="mono ed" bind:value={text} rows="18" aria-label="Proposed AGENTS.md"></textarea>
          {#if edited}<p class="hint">Edited: your text replaces the agent's. <button class="btn ghost sm" onclick={() => (text = view?.proposed ?? '')}>Undo edits</button></p>{/if}
        {:else}
          <p class="hint mono">AGENTS.md{view.current == null ? ' (new file)' : ''}</p>
          <DiffLines lines={agents} />
          {#if claude}
            <p class="hint mono">CLAUDE.md{view.claude?.current == null ? ' (new file)' : ''}: imports AGENTS.md so Claude Code reads it</p>
            <DiffLines lines={claude} />
          {/if}
        {/if}
      {/if}
      {#if blocked}<Callout tone="info">{blocked}</Callout>{/if}
    {/if}
    {#snippet foot()}
      {#if view?.proposal}
        <button class="btn ghost" disabled={busy} onclick={() => act('memory-dismiss')}>Dismiss</button>
        <button class="btn primary" disabled={busy || !!blocked || view.proposed == null || !text.trim()} onclick={() => act('memory-accept')}>
          Accept and commit</button>
      {/if}
    {/snippet}
  </Panel>
</div>

<style>
  .ed { width: 100%; min-height: 18em; }
</style>
