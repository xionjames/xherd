<script lang="ts">
  // Action page (`?s=act-<id>`, R14, R16, R25): what the action is, Run / Stop /
  // Restart, its terminal while it runs, the saved output once it ended, and
  // Save to repo profile. Every text is plain: the command is always shown in
  // full, never only the label.
  import TermView from '../components/TermView.svelte';
  import Dialog from '../components/Dialog.svelte';
  import Callout from '../components/Callout.svelte';
  import type { ActionsView, Job } from '../lib/types';
  import { get, post } from '../lib/api';
  import { upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import { lastResult, needsApproval, running } from '../lib/actions';

  let { agent, actionId }: { agent: Job; actionId: string } = $props();
  // Keyed on ids: the agent object is replaced on every update.
  const id = $derived(agent.id);
  const act = $derived(agent.actions?.find((a) => a.def.id === actionId));
  const isRunning = $derived(!!act && running(act));
  const result = $derived(act ? lastResult(act) : undefined);
  const live = $derived(['running', 'waiting-input', 'blocked', 'idle', 'paused', 'parked', 'watching', 'error'].includes(agent.status));

  let busy = $state(false);
  let confirm = $state<'run' | 'restart' | null>(null);

  async function call(verb: 'run' | 'stop' | 'restart', approve = false) {
    busy = true;
    try {
      upsertJob((await post<{ job: Job }>(`/jobs/${id}/actions/${actionId}/${verb}`, approve ? { approve: true } : {})).job);
    } catch (e) { showError(`Could not ${verb} the action`, e); }
    finally { busy = false; }
  }
  // The first run of a command nobody allowed asks first (R14).
  function start(verb: 'run' | 'restart') {
    if (act && needsApproval(act)) confirm = verb;
    else call(verb);
  }
  function allow() {
    const verb = confirm;
    confirm = null;
    if (verb) call(verb, true);
  }

  // The saved output of an ended run (R13): read again when a run ends.
  let log = $state<string | null>(null);
  let logErr = $state('');
  const ranAt = $derived(act?.last?.endedAt ?? 0);
  $effect(() => {
    const aid = actionId, jid = id;
    void ranAt;
    if (!act?.last || isRunning) { log = null; logErr = ''; return; }
    let stale = false;
    get<{ log: string | null }>(`/jobs/${jid}/actions/${aid}/log`)
      .then((r) => { if (!stale) { log = r.log; logErr = ''; } })
      .catch((e) => { if (!stale) logErr = e instanceof Error ? e.message : String(e); });
    return () => { stale = true; };
  });

  // Save to repo profile (R22): offered for a job action when the repo has a profile.
  let hasProfile = $state(false);
  $effect(() => {
    const jid = id;
    void act?.source;
    let stale = false;
    get<ActionsView>(`/jobs/${jid}/actions`).then((v) => { if (!stale) hasProfile = v.hasProfile; }).catch(() => {});
    return () => { stale = true; };
  });
  async function save() {
    busy = true;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${id}/actions/${actionId}/save-to-profile`)).job); }
    catch (e) { showError('Could not save the action to the repository profile', e); }
    finally { busy = false; }
  }
  const when = (ms?: number) => (ms ? new Date(ms).toLocaleString() : '');
</script>

{#if !act}
  <Callout tone="plain">This action no longer exists.</Callout>
{:else}
  <div class="stack">
    <section class="card act-head">
      <div class="row" style="justify-content:space-between">
        <h2 class="atitle">{act.def.label || act.def.id}</h2>
        <span class="tc" title={result?.text}><span aria-hidden="true">{result?.icon}</span> {result?.text}</span>
      </div>
      {#if act.def.description}<p class="adesc">{act.def.description}</p>{/if}
      <dl class="afacts">
        <dt>Command</dt><dd><code class="mono acmd">{act.def.run}</code></dd>
        <dt>Directory</dt><dd class="mono">{act.def.cwd || '(worktree root)'}</dd>
        <dt>Kind</dt><dd>{act.def.kind === 'service' ? 'Service (keeps running)' : 'Task (runs once)'}</dd>
        {#if act.source === 'profile'}<dt>From</dt><dd>The repository profile</dd>{/if}
        {#if act.last}<dt>Last run</dt><dd>started {when(act.last.startedAt)}{#if act.last.endedAt}, ended {when(act.last.endedAt)}{/if}</dd>{/if}
      </dl>
      <div class="row">
        {#if isRunning}
          <button class="btn" disabled={busy} onclick={() => call('stop')}>Stop</button>
          <button class="btn" disabled={busy} onclick={() => start('restart')}>Restart</button>
        {:else}
          <button class="btn primary" disabled={busy || !live} onclick={() => start('run')}>{needsApproval(act) ? 'Allow and run…' : 'Run'}</button>
          {#if act.last}<button class="btn" disabled={busy || !live} onclick={() => start('restart')}>Restart</button>{/if}
        {/if}
        {#if act.source === 'job' && hasProfile}
          <button class="btn ghost" disabled={busy} title="Every new job in this repository gets this action" onclick={save}>Save to repo profile</button>
        {/if}
      </div>
      {#if !live && !isRunning}<span class="hint">The job is {agent.status}: actions cannot start.</span>{/if}
    </section>

    {#if isRunning}
      <TermView path={`/ws/term/${id}?session=act-${actionId}`} title={act.def.label || act.def.id}
        hint="The action's own tmux session; leaving this page detaches, it keeps running." />
    {:else if act.last}
      <section class="card alog">
        <div class="lbl">Output{#if act.last.state === 'exited'} · exit code {act.last.exit ?? 0}{:else} · {result?.text.toLowerCase()}{/if}</div>
        {#if logErr}<span class="hint">Could not read the output: {logErr}</span>
        {:else if log === null}<span class="hint">No saved output.</span>
        {:else}<pre class="mono alogtext">{log}</pre>{/if}
      </section>
    {/if}
  </div>
{/if}

{#if confirm && act}
  <Dialog title="Allow this command?" onclose={() => (confirm = null)}>
    <p>{agent.name} registered this command. It runs in the job's worktree with your permissions once you allow it.</p>
    <dl class="afacts">
      <dt>Command</dt><dd><code class="mono acmd">{act.def.run}</code></dd>
      <dt>Directory</dt><dd class="mono">{act.def.cwd || '(worktree root)'}</dd>
      <dt>Kind</dt><dd>{act.def.kind === 'service' ? 'Service' : 'Task'}</dd>
    </dl>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirm = null)}>Cancel</button>
      <button class="btn primary" onclick={allow}>Allow and run</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .act-head { display: flex; flex-direction: column; gap: 10px; padding: 16px; }
  .atitle { margin: 0; font-size: 1.1rem; overflow-wrap: anywhere; }
  .adesc { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .afacts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; margin: 0; }
  .afacts dt { color: var(--muted, inherit); }
  .afacts dd { margin: 0; overflow-wrap: anywhere; }
  .acmd { white-space: pre-wrap; word-break: break-all; }
  .alog { padding: 16px; display: flex; flex-direction: column; gap: 8px; }
  .alogtext { margin: 0; max-height: 60vh; overflow: auto; white-space: pre-wrap; word-break: break-all; }
</style>
