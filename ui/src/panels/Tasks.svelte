<script lang="ts">
  import Panel from '../components/Panel.svelte';
  import type { Agent } from '../lib/types';
  let { agent }: { agent: Agent } = $props();
  const GLYPH = { todo: '○', doing: '●', done: '✓', blocked: '✕' };
  const TONE = { todo: '', doing: 'acc', done: 'live', blocked: 'halt' };
</script>

<div class="stack">
  <Panel title="Tasks" flush>
    {#snippet head()}<span class="pill num">{(agent.tasks ?? []).filter((t) => t.status === 'done').length}/{agent.tasks?.length ?? 0} done</span>{/snippet}
    {#if agent.tasks?.length}
      <div class="tscroll">
        <table class="tbl">
          <thead><tr><th>Task</th><th>Status</th><th class="hide-sm">Files</th><th class="hide-sm">Evidence</th></tr></thead>
          <tbody>
            {#each agent.tasks as t (t.id)}
              <tr>
                <td><span class="mono muted">{t.id}</span> {t.title}
                  {#if t.depends.length}<div class="hint">after {t.depends.join(', ')}</div>{/if}</td>
                <td><span class="pill {TONE[t.status]}">{GLYPH[t.status]} {t.status}</span>
                  {#if t.blockedReason}<div class="hint err">{t.blockedReason}</div>{/if}</td>
                <td class="mono muted hide-sm">{t.files.join(', ')}</td>
                <td class="muted hide-sm">{t.evidence ?? ''}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="cb"><p class="empty">No tasks yet: they appear when the agent writes and syncs its task list.</p></div>
    {/if}
  </Panel>
</div>
