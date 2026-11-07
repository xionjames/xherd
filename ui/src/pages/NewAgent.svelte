<script lang="ts">
  // New agent on a ticket (POST /agents).
  import { post } from '../lib/api';
  import { upsertAgent, app } from '../lib/store.svelte';
  import { go } from '../lib/router.svelte';
  import { showError } from '../lib/errors.svelte';
  import type { Agent } from '../lib/types';

  const flows = $derived([...new Set((app.state?.flows ?? []).map((f) => f.flow))]);
  let f = $state({ ticketKey: '', slug: '', repo: '', base: '', flow: 'feature', prompt: '' });
  let busy = $state(false);
  const keyOk = $derived(/^[A-Z0-9][A-Z0-9_-]*$/.test(f.ticketKey));
  const slugOk = $derived(!f.slug || /^[a-z0-9][a-z0-9-]{0,40}$/.test(f.slug));
  const name = $derived(f.ticketKey + (f.slug ? `-${f.slug}` : ''));
  const ready = $derived(keyOk && slugOk && f.repo.startsWith('/') && (f.flow !== 'none' || f.prompt.trim() !== ''));

  async function create(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const body: Record<string, string> = { ticketKey: f.ticketKey, repo: f.repo.trim(), flow: f.flow };
    if (f.slug) body.slug = f.slug;
    if (f.base.trim()) body.base = f.base.trim();
    if (f.prompt.trim()) body.prompt = f.prompt.trim();
    try {
      const r = await post<{ agent: Agent }>('/agents', body);
      upsertAgent(r.agent);
      go(`/agent/${r.agent.name}`);
    } catch (err) {
      showError('Could not create the agent', err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="phead notabs"><h1>New agent</h1><p class="muted">One ticket, one worktree, one agent.</p></div>
<form class="stack" style="max-width:860px" onsubmit={create}>
  <section class="sec"><div class="bd">
    <div class="g2">
      <label class="f">Ticket key
        <input class="mono" bind:value={f.ticketKey} placeholder="PAY-412" required oninput={() => (f.ticketKey = f.ticketKey.toUpperCase())} />
        {#if f.ticketKey && !keyOk}<span class="hint" style="color:var(--halt)">Capital letters, digits, - and _.</span>{/if}</label>
      <label class="f">Short name (optional)
        <input class="mono" bind:value={f.slug} placeholder="refund-retry" />
        {#if !slugOk}<span class="hint" style="color:var(--halt)">Lowercase letters, digits and -, up to 41.</span>
        {:else if f.ticketKey}<span class="hint">Agent, branch and session: <span class="mono">{name}</span></span>{/if}</label>
    </div>
    <div class="g2">
      <label class="f">Repository (absolute path)
        <input class="mono" bind:value={f.repo} placeholder="/Users/you/src/payments-api" required /></label>
      <label class="f">Base branch (optional)
        <input class="mono" bind:value={f.base} placeholder="origin's default branch" /></label>
    </div>
    <div class="f">Workflow
      <div class="seg" role="group" aria-label="Workflow" style="align-self:flex-start">
        {#each [...flows, 'none'] as fl (fl)}
          <button type="button" class:on={f.flow === fl} aria-pressed={f.flow === fl} onclick={() => (f.flow = fl)}>{fl === 'none' ? 'No workflow' : fl}</button>
        {/each}
      </div></div>
    <label class="f">{f.flow === 'none' ? 'Prompt' : 'What to do (optional)'}
      <textarea rows="4" bind:value={f.prompt} placeholder={f.flow === 'none' ? 'Typed into the agent as its first message' : 'Context for the spec step'}></textarea></label>
  </div></section>
  <div class="row"><span class="grow"></span>
    <button type="button" class="btn ghost" onclick={() => history.back()}>Cancel</button>
    <button class="btn primary lg" disabled={!ready || busy}>Create agent</button></div>
</form>
