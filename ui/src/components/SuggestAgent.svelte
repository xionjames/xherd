<script lang="ts">
  // "Save this setup as an agent?" (R30…R32): a band on the job page when the
  // daemon suggested it, and the form it opens, which "Save as agent" in the
  // job's menu opens too. Nothing is saved until Save; never a modal on its
  // own and it never blocks the job.
  import { post, put } from '../lib/api';
  import { go } from '../lib/router.svelte';
  import { upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { fillFromJob, fromForm, suggestionOpen, type AgentForm } from '../lib/agents';
  import Callout from './Callout.svelte';
  import Dialog from './Dialog.svelte';
  import type { Job } from '../lib/types';

  let { agent, open = $bindable(false) }: { agent: Job; open?: boolean } = $props();
  const id = $derived(agent.id);
  const offered = $derived(suggestionOpen(agent));
  let form = $state<AgentForm | null>(null);
  let busy = $state(false);

  function start() {
    form = fillFromJob(agent);
    open = true;
  }
  // The menu opens the form through `open`.
  $effect(() => {
    if (open && !form) form = fillFromJob(agent);
    if (!open) form = null;
  });

  async function save() {
    if (!form) return;
    busy = true;
    try {
      const path = offered ? `/jobs/${id}/suggestion/accept` : `/jobs/${id}/save-as-agent`;
      const r = await post<{ agent: { name: string }; job: Job }>(path, { agent: fromForm(form) });
      upsertJob(r.job);
      open = false;
      go(`/agents/${r.agent.name}`);
    } catch (e) {
      showError('Could not save the agent', e);
    } finally {
      busy = false;
    }
  }
  async function dismiss() {
    try { upsertJob((await post<{ job: Job }>(`/jobs/${id}/suggestion/dismiss`)).job); }
    catch (e) { showError('Could not dismiss', e); }
  }
  async function never() {
    try {
      await put('/config', { suggestAgents: false });
      await dismiss();
    } catch (e) { showError('Could not turn suggestions off', e); }
  }
  const valid = $derived(!!form && /^[a-z0-9][a-z0-9-]{0,40}$/.test(form.name));
</script>

{#if offered}
  <Callout tone="info">
    Save this setup as an agent? Next time you start a job from it, the runtime and autonomy are already filled in.
    <button class="btn sm" onclick={start}>Save as agent…</button>
    <button class="btn ghost sm" onclick={dismiss}>Not now</button>
    <button class="btn ghost sm" onclick={never}>Don't suggest</button>
  </Callout>
{/if}

{#if open && form}
  <Dialog title="Save as agent" onclose={() => (open = false)}>
    <label class="f">Name
      <input class="mono" bind:value={form.name} aria-invalid={!valid} />
      <span class="hint">Lowercase letters, digits and -; fixed after saving.</span></label>
    <label class="f">Description
      <input bind:value={form.description} maxlength="200" /></label>
    <label class="f">Instructions
      <textarea rows="6" bind:value={form.instructions} placeholder="What every job of this agent should know"></textarea>
      <span class="hint">{agent.agentDraft ? 'The job drafted these from what you told it; edit freely.' : 'Optional.'} Nothing task-specific.</span></label>
    <p class="hint">
      Runtime <b>{form.runtime}</b>{form.model ? ` (${form.model})` : ''},
      {form.autonomy ? `autonomy ${form.autonomy}` : 'autonomy from Settings'}{form.permissionMode ? `, tools ${form.permissionMode}` : ''}.
      This job keeps its own settings and becomes the agent's first job.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (open = false)}>Cancel</button>
      <button class="btn primary" disabled={!valid || busy} onclick={save}>Save agent</button>
    {/snippet}
  </Dialog>
{/if}
