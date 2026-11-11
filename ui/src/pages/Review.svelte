<script lang="ts">
  // To review (R-111): what xherd approved by itself, newest first. "Mark seen"
  // hides an item (it stays in the agent's approval ledger, the full trail).
  import { app } from '../lib/store.svelte';
  import { get, post } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';

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

<div class="stack">
  <div class="phead notabs"><div class="row"><h1>To review</h1>
    <span class="grow"></span>
    <label class="hint"><input type="checkbox" bind:checked={showSeen} /> Show seen</label>
    <button class="btn sm" disabled={!unseen.length} onclick={() => markSeen(unseen)}>Mark all seen</button></div></div>
  <p class="muted">Gates xherd approved by itself under autonomy <span class="mono">important</span>. Anything a tripwire held is not here: it waited for you.</p>
  {#if items === null}
    <p class="muted">Loading…</p>
  {:else if shown.length === 0}
    <p class="muted">{items.length ? 'Everything is seen.' : 'Nothing yet.'}</p>
  {:else}
    <ul class="card" style="list-style:none;margin:0;padding:0">
      {#each shown as it (`${it.agentId}${it.at}${it.gate}`)}
        <li style="padding:12px 16px;border-bottom:1px solid var(--fence)">
          <div class="row" style="gap:10px">
            {#if name(it.agentId)}<a href={href(`/agent/${name(it.agentId)}`)}><b>{name(it.agentId)}</b></a>{:else}<span class="muted">removed agent</span>{/if}
            <span class="pill mono" style="text-transform:none">{it.gate}</span>
            <span class="hint">{agoText(it.at)}</span>
            <span class="grow"></span>
            {#if it.seen}<span class="hint">seen</span>{:else}<button class="btn ghost sm" onclick={() => markSeen([it])}>Mark seen</button>{/if}
          </div>
          {#if it.reason}<div class="hint">{it.reason}</div>{/if}
          {#if it.files?.length}<div class="mono hint">{it.files.join(', ')}</div>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
