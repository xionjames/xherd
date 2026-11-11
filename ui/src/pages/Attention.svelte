<script lang="ts">
  // Needs attention: everything waiting on you in one place. Agents at a gate,
  // blocked or asking (oldest first); TODOs that are due or whose check
  // failed; and what xherd approved by itself (R-111), newest first, until
  // marked seen ("Mark seen" hides an item; the approval ledger keeps it).
  import { app } from '../lib/store.svelte';
  import { post } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';
  import { attention, refresh, waitingAgents, dueTodos, unseenApprovals, type AutoApproval } from '../lib/attention.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import NeedsCards from '../components/NeedsCards.svelte';

  let showSeen = $state(false);
  $effect(() => { refresh(); });

  const waiting = $derived(waitingAgents());
  const todos = $derived(dueTodos());
  const unseen = $derived(unseenApprovals());
  const approvals = $derived((attention.reviews ?? []).filter((i) => showSeen || !i.seen));
  const nothing = $derived(!waiting.length && !todos.length && !unseen.length);

  async function markSeen(list: AutoApproval[]) {
    try {
      await post('/reviews/seen', { items: list.map(({ agentId, at, gate }) => ({ agentId, at, gate })) });
      await refresh();
    } catch (e) { showError('Could not mark them seen', e); }
  }
  const name = (id: string) => app.state?.agents.find((a) => a.id === id)?.name;
</script>

<PageHead title="Needs attention">
  {#snippet sub()}Agents waiting for you, TODOs that are due, and gates xherd approved by itself.{/snippet}
</PageHead>

<div class="stack">
  {#if nothing}
    <p class="empty">Nothing needs you right now.</p>
  {/if}

  {#if waiting.length}
    <div class="sect"><h2>Agents waiting for you</h2><span class="hint">oldest first</span></div>
    <NeedsCards agents={waiting} />
  {/if}

  {#if todos.length}
    <Panel title="TODOs" hint="due, or the check failed" flush>
      <ul class="list">
        {#each todos as t (t.id)}
          <li>
            <div class="row">
              <a href={href('/todos')}><b>{t.title}</b></a>
              <span class="pill" class:gate={t.status === 'due'} class:halt={t.status === 'error'}>{t.status === 'error' ? 'check failed' : 'due'}</span>
              <span class="grow"></span>
              <span class="hint">{agoText(t.due)}</span>
            </div>
          </li>
        {/each}
      </ul>
    </Panel>
  {/if}

  {#if attention.reviews?.length}
    <Panel title="Approved by xherd" hint="under autonomy important; anything a tripwire held waited for you instead" flush>
      {#snippet head()}
        <label class="f chk"><input type="checkbox" bind:checked={showSeen} /> Show seen</label>
        <button class="btn sm" disabled={!unseen.length} onclick={() => markSeen(unseen)}>Mark all seen</button>
      {/snippet}
      {#if approvals.length}
        <ul class="list">
          {#each approvals as it (`${it.agentId}${it.at}${it.gate}`)}
            <li>
              <div class="row">
                {#if name(it.agentId)}<a href={href(`/agent/${name(it.agentId)}`)}><b>{name(it.agentId)}</b></a>{:else}<span class="muted">removed agent</span>{/if}
                <span class="pill mono">{it.gate}</span>
                <span class="hint">{agoText(it.at)}</span>
                <span class="grow"></span>
                {#if it.seen}<span class="hint">seen</span>{:else}<button class="btn ghost sm" onclick={() => markSeen([it])}>Mark seen</button>{/if}
              </div>
              {#if it.reason}<div class="hint">{it.reason}</div>{/if}
              {#if it.files?.length}<div class="mono hint wrap-any">{it.files.join(', ')}</div>{/if}
            </li>
          {/each}
        </ul>
      {:else}
        <div class="cb"><p class="empty">Everything is seen.</p></div>
      {/if}
    </Panel>
  {/if}
</div>
