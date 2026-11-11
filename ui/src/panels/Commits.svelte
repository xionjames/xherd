<script lang="ts">
  import Panel from '../components/Panel.svelte';
  // Commits of the agent: click one for its message, files and the diff.
  import type { Agent, Commit } from '../lib/types';
  import { href } from '../lib/router.svelte';
  let { agent }: { agent: Agent } = $props();
  const TONE = { proposed: 'gate', approved: 'acc', done: 'live', dropped: '' };
  const GLYPH = { proposed: '✋', approved: '✓', done: '✓', dropped: '–' };
  let picked = $state<string | null>(null);
  // Default to the open proposal, else the latest commit.
  const open = $derived(agent.gate?.kind === 'commit' ? agent.gate.id : null);
  const selected = $derived<Commit | undefined>(
    agent.commits?.find((c) => c.id === (picked ?? open)) ?? agent.commits?.[agent.commits.length - 1],
  );
</script>

<div class="stack">
  <Panel title="Commits" flush>
    {#if agent.commits?.length}
      <div class="tscroll">
        <table class="tbl">
          <thead><tr><th>Commit</th><th>Status</th><th class="hide-sm">Files</th><th class="hide-sm">Tasks</th><th class="hide-sm">SHA</th></tr></thead>
          <tbody>
            {#each agent.commits as c (c.id)}
              <tr class="hov" class:on={c.id === selected?.id} onclick={() => (picked = c.id)}
                tabindex="0" onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (picked = c.id)} aria-selected={c.id === selected?.id}>
                <td><span class="mono muted">{c.id}</span> <span class="mono">{c.subject}</span>
                  {#if c.needsHuman}<div><span class="pill gate">held: {c.needsHuman}</span></div>{/if}</td>
                <td><span class="pill {c.status === 'proposed' && agent.gate?.kind !== 'commit' ? '' : TONE[c.status]}">{GLYPH[c.status]} {c.status}</span></td>
                <td class="num muted hide-sm">{c.files.length}</td>
                <td class="mono muted hide-sm">{c.tasks.join(', ')}</td>
                <td class="mono muted hide-sm">{c.sha?.slice(0, 8) ?? ''}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="cb"><p class="empty">No commits proposed yet.</p></div>
    {/if}
  </Panel>

  {#if selected}
    <Panel label={selected.id}>
      {#snippet head()}
        <h2 class="grow wrap-any"><span class="mono muted">{selected.id}</span> {selected.subject}</h2>
        <a class="btn sm" href={href(`/agent/${agent.name}`, { s: 'changes', view: 'commit', commit: selected.id })}>View the changes →</a>
      {/snippet}
      <div>
        <span class="lbl">Commit message</span>
        <pre class="mono pre msg">{selected.message ?? selected.subject}</pre>
      </div>
      <div>
        <span class="lbl">Files · {selected.files.length}</span>
        <ul class="mono files">{#each selected.files as f (f)}<li class="wrap-any">{f}</li>{/each}</ul>
      </div>
      {#if selected.tasks.length}<div class="hint">Tasks: <span class="mono">{selected.tasks.join(', ')}</span></div>{/if}
      {#if selected.sha}<div class="hint wrap-any">Committed as <span class="mono">{selected.sha}</span></div>{/if}
    </Panel>
  {/if}
</div>

<style>
  .msg { margin: 6px 0 0; font-size: 13px; line-height: 1.45; }
  .files { font-size: 12.5px; margin: 6px 0 0; }
</style>
