<script lang="ts">
  // To review (R-111): what xherd approved by itself, newest first. "Mark seen"
  // hides an item (it stays in the agent's approval ledger, the full trail).
  import { app } from '../lib/store.svelte';
  import { get, post } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';

  type Item = { agentId: string; at: number; gate: string; files: string[]; reason?: string; seen: boolean };
  let items = $state<Item[] | null>(null);
  let showSeen = $state(false);
  const load = () => get<{ reviews: Item[] }>('/reviews').then((r) => (items = r.reviews)).catch((e) => showError('Could not load the list', e));
  $effect(() => { load(); });
  const shown = $derived((items ?? []).filter((i) => showSeen || !i.seen));
  const unseen = $derived((items ?? []).filter((i) => !i.seen));
  async function markSeen(list: Item[]) {
    try {
      await post('/reviews/seen', { items: list.map(({ agentId, at, gate }) => ({ agentId, at, gate })) });
      await load();
    } catch (e) { showError('Could not mark them seen', e); }
  }
  const name = (id: string) => app.state?.agents.find((a) => a.id === id)?.name;
</script>

<PageHead title="To review">
  {#snippet sub()}Gates xherd approved by itself under autonomy <span class="mono">important</span>. Anything a tripwire held is not here: it waited for you.{/snippet}
  {#snippet actions()}
    <label class="f chk"><input type="checkbox" bind:checked={showSeen} /> Show seen</label>
    <button class="btn" disabled={!unseen.length} onclick={() => markSeen(unseen)}>Mark all seen</button>
  {/snippet}
</PageHead>

<div class="stack mid">
  {#if items === null}
    <p class="empty">Loading…</p>
  {:else if shown.length === 0}
    <p class="empty">{items.length ? 'Everything is seen.' : 'Nothing yet.'}</p>
  {:else}
    <Panel label="Approved by xherd" flush>
      <ul class="list">
        {#each shown as it (`${it.agentId}${it.at}${it.gate}`)}
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
    </Panel>
  {/if}
</div>
