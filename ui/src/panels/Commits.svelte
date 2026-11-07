<script lang="ts">
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
  <section class="card">
    <div class="ch"><h2>Commits</h2></div>
    {#if agent.commits?.length}
      <table class="tbl">
        <thead><tr><th>Commit</th><th>Status</th><th>Files</th><th>Tasks</th><th>SHA</th></tr></thead>
        <tbody>
          {#each agent.commits as c (c.id)}
            <tr class:on={c.id === selected?.id} style="cursor:pointer" onclick={() => (picked = c.id)}
              tabindex="0" onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (picked = c.id)} aria-selected={c.id === selected?.id}>
              <td><span class="mono muted">{c.id}</span> <span class="mono">{c.subject}</span>
                {#if c.needsHuman}<div class="hint" style="color:var(--gate)">held: {c.needsHuman}</div>{/if}</td>
              <td><span class="pill {c.status === 'proposed' && agent.gate?.kind !== 'commit' ? '' : TONE[c.status]}">{GLYPH[c.status]} {c.status}</span></td>
              <td class="num muted">{c.files.length}</td>
              <td class="mono muted">{c.tasks.join(', ')}</td>
              <td class="mono muted">{c.sha?.slice(0, 8) ?? ''}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="cb muted">No commits proposed yet.</div>
    {/if}
  </section>

  {#if selected}
    <section class="card">
      <div class="ch">
        <h2><span class="mono muted">{selected.id}</span> {selected.subject}</h2>
        <span class="grow"></span>
        <a class="btn sm" href={href(`/agent/${agent.name}`, { s: 'changes', view: 'commit', commit: selected.id })}>View the changes →</a>
      </div>
      <div class="cb stack" style="gap:12px">
        <div>
          <span class="lbl">Commit message</span>
          <pre class="mono" style="white-space:pre-wrap;margin:6px 0 0;font-size:13px;line-height:1.45">{selected.message ?? selected.subject}</pre>
        </div>
        <div>
          <span class="lbl">Files · {selected.files.length}</span>
          <ul class="mono" style="font-size:12.5px;margin:6px 0 0">{#each selected.files as f (f)}<li>{f}</li>{/each}</ul>
        </div>
        {#if selected.tasks.length}<div class="hint">Tasks: <span class="mono">{selected.tasks.join(', ')}</span></div>{/if}
        {#if selected.sha}<div class="hint">Committed as <span class="mono">{selected.sha}</span></div>{/if}
      </div>
    </section>
  {/if}
</div>
