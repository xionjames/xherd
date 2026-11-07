<script lang="ts">
  import type { Agent } from '../lib/types';
  let { agent }: { agent: Agent } = $props();
  const TONE = { proposed: 'gate', approved: 'acc', done: 'live', dropped: '' };
  const GLYPH = { proposed: '✋', approved: '✓', done: '✓', dropped: '–' };
</script>

<div class="stack">
  <section class="card">
    <div class="ch"><h2>Commits</h2></div>
    {#if agent.commits?.length}
      <table class="tbl">
        <thead><tr><th>Commit</th><th>Status</th><th>Files</th><th>Tasks</th><th>SHA</th></tr></thead>
        <tbody>
          {#each agent.commits as c (c.id)}
            <tr>
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
</div>
