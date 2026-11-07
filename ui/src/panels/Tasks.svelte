<script lang="ts">
  import type { Agent } from '../lib/types';
  let { agent }: { agent: Agent } = $props();
  const GLYPH = { todo: '○', doing: '●', done: '✓', blocked: '✕' };
  const TONE = { todo: '', doing: 'acc', done: 'live', blocked: 'halt' };
</script>

<div class="stack">
  <section class="card">
    <div class="ch"><h2>Tasks</h2><span class="muted">{(agent.tasks ?? []).filter((t) => t.status === 'done').length}/{agent.tasks?.length ?? 0} done</span></div>
    {#if agent.tasks?.length}
      <table class="tbl">
        <thead><tr><th>Task</th><th>Status</th><th>Files</th><th>Evidence</th></tr></thead>
        <tbody>
          {#each agent.tasks as t (t.id)}
            <tr>
              <td><span class="mono muted">{t.id}</span> {t.title}
                {#if t.depends.length}<div class="hint">after {t.depends.join(', ')}</div>{/if}</td>
              <td><span class="pill {TONE[t.status]}">{GLYPH[t.status]} {t.status}</span>
                {#if t.blockedReason}<div class="hint" style="color:var(--halt)">{t.blockedReason}</div>{/if}</td>
              <td class="mono muted">{t.files.join(', ')}</td>
              <td class="muted">{t.evidence ?? ''}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <div class="cb muted">No tasks yet: they appear when the agent writes and syncs its task list.</div>
    {/if}
  </section>
</div>
