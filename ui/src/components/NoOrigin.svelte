<script lang="ts">
  // "No remote" card (R9, R10): the job stopped at the ship step because the
  // repository has no `origin`. The user adds one (push and PR follow) or
  // finishes the job locally. Errors show inline, from the daemon's refusal.
  import type { Job } from '../lib/types';
  import { ApiError, post } from '../lib/api';
  import { upsertJob } from '../lib/store.svelte';
  import Callout from './Callout.svelte';

  let { agent }: { agent: Job } = $props();
  const id = $derived(agent.id);

  let url = $state('');
  let busy = $state(false);
  let error = $state('');

  async function run(path: string, body?: unknown) {
    busy = true;
    error = '';
    try { upsertJob((await post<{ job: Job }>(`/jobs/${id}/${path}`, body)).job); }
    catch (e) { error = e instanceof ApiError ? `${e.message} (${e.code})` : String(e); }
    finally { busy = false; }
  }
</script>

<section class="card" aria-label="No remote">
  <div class="ch"><div class="t"><b>This repository has no remote</b>
    <span class="muted">The branch is ready but there is nowhere to push it. Add a remote to push and open a pull request, or finish the job here.</span></div></div>
  <div class="cb">
    <label class="f">Remote URL
      <input class="mono" bind:value={url} placeholder="https://github.com/acme/app.git" autocomplete="off" disabled={busy} />
    </label>
    {#if error}<Callout tone="bad">{error}</Callout>{/if}
  </div>
  <div class="cf">
    <button class="btn ghost" disabled={busy} onclick={() => run('finish-local')}>Finish locally</button>
    <button class="btn primary" disabled={busy || !url.trim()} onclick={() => run('set-origin', { url: url.trim() })}>Set origin and open PR</button>
  </div>
</section>
