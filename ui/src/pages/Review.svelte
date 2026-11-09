<script lang="ts">
  // To review (R-111): what xherd approved by itself, newest first. Read-only;
  // the full trail is each agent's approval ledger.
  import { app } from '../lib/store.svelte';
  import { get } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { agoText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';

  type Item = { agentId: string; at: number; gate: string; files: string[]; reason?: string };
  let items = $state<Item[] | null>(null);
  $effect(() => {
    get<{ reviews: Item[] }>('/reviews').then((r) => (items = r.reviews)).catch((e) => showError('Could not load the list', e));
  });
  const name = (id: string) => app.state?.agents.find((a) => a.id === id)?.name;
</script>

<div class="stack">
  <div class="phead notabs"><div class="row"><h1>To review</h1></div></div>
  <p class="muted">Gates xherd approved by itself under autonomy <span class="mono">important</span>. Anything a tripwire held is not here: it waited for you.</p>
  {#if items === null}
    <p class="muted">Loading…</p>
  {:else if items.length === 0}
    <p class="muted">Nothing yet.</p>
  {:else}
    <ul class="card" style="list-style:none;margin:0;padding:0">
      {#each items as it (`${it.agentId}${it.at}${it.gate}`)}
        <li style="padding:12px 16px;border-bottom:1px solid var(--fence)">
          <div class="row" style="gap:10px">
            {#if name(it.agentId)}<a href={href(`/agent/${name(it.agentId)}`)}><b>{name(it.agentId)}</b></a>{:else}<span class="muted">removed agent</span>{/if}
            <span class="pill mono" style="text-transform:none">{it.gate}</span>
            <span class="hint">{agoText(it.at)}</span>
          </div>
          {#if it.reason}<div class="hint">{it.reason}</div>{/if}
          {#if it.files?.length}<div class="mono hint">{it.files.join(', ')}</div>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
