<script lang="ts">
  // "Save actions to repo profile" card (R23): once a job is done or has its
  // pull request, offers the actions that succeeded and are not on the repo
  // profile yet, all checked. Saving writes the checked ones, dismissing hides
  // the card for this job.
  import type { ActionsView, Job } from '../lib/types';
  import { get, post } from '../lib/api';
  import { upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { saveCard } from '../lib/actions';

  let { agent }: { agent: Job } = $props();
  // Keyed on ids and a signature of the runs: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const ended = $derived(agent.status === 'done' || !!agent.prUrl);
  const sig = $derived((agent.actions ?? []).map((a) => `${a.def.id}:${a.succeeded ? 1 : 0}:${a.last?.endedAt ?? 0}:${a.source}`).join(','));

  let view = $state<ActionsView | null>(null);
  let unchecked = $state<string[]>([]);
  let busy = $state(false);

  $effect(() => {
    const jid = id;
    void sig;
    if (!ended || agent.actionsCardDismissed) { view = null; return; }
    let stale = false;
    get<ActionsView>(`/jobs/${jid}/actions`).then((v) => { if (!stale) view = v; }).catch(() => { if (!stale) view = null; });
    return () => { stale = true; };
  });

  const list = $derived(ended ? saveCard(view?.actions, view?.saveOffer ?? [], agent.actionsCardDismissed) : []);
  const chosen = $derived(list.filter((a) => !unchecked.includes(a.def.id)).map((a) => a.def.id));

  function toggle(aid: string, on: boolean) {
    unchecked = on ? unchecked.filter((x) => x !== aid) : [...unchecked, aid];
  }
  async function save() {
    busy = true;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${id}/actions/save-to-profile`, { ids: chosen })).job); }
    catch (e) { showError('Could not save the actions', e); }
    finally { busy = false; }
  }
  async function dismiss() {
    busy = true;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${id}/actions/card-dismiss`)).job); }
    catch (e) { showError('Could not dismiss the card', e); }
    finally { busy = false; }
  }
</script>

{#if list.length}
  <section class="card" aria-label="Save actions to repo profile">
    <div class="ch"><div class="t"><b>Save actions to repo profile</b>
      <span class="muted">These ran well in this job. Every new job in this repository gets the ones you save.</span></div></div>
    <div class="cb tight">
      {#each list as a (a.def.id)}
        <label class="row">
          <input type="checkbox" checked={!unchecked.includes(a.def.id)} disabled={busy} onchange={(e) => toggle(a.def.id, e.currentTarget.checked)} />
          <span><b>{a.def.label || a.def.id}</b> <span class="mono">{a.def.run}</span></span>
        </label>
      {/each}
    </div>
    <div class="cf">
      <button class="btn ghost" disabled={busy} onclick={dismiss}>Dismiss</button>
      <button class="btn primary" disabled={busy || !chosen.length} onclick={save}>Save {chosen.length}</button>
    </div>
  </section>
{/if}
