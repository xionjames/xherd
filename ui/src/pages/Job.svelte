<script lang="ts">
  // Job page: your-call band, hero (name, meta, actions, progress), tabs of
  // panels from the registry.
  import Icon from '../components/Icon.svelte';
  import TicketPill from '../components/TicketPill.svelte';
  import AgentDot from '../components/AgentDot.svelte';
  import Stepper from '../components/Stepper.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import YourCallBand from '../components/YourCallBand.svelte';
  import Dialog from '../components/Dialog.svelte';
  import Callout from '../components/Callout.svelte';
  import TicketPrompt from '../components/TicketPrompt.svelte';
  import type { Job } from '../lib/types';
  import { app, upsertJob } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { PANELS, agentTabs } from '../lib/panels';
  import { flowOf, prGate } from '../lib/flow';
  import { agoText, needsYou, repoName, shortBase, waitsAtGate } from '../lib/format';
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { MODE_HINT, modes } from '../lib/modes';
  import ModelInput from '../components/ModelInput.svelte';
  import { canChooseModel, validModel } from '../lib/models';
  import { agentOf, changedSince, waitsForAgent } from '../lib/agents';

  let { agent }: { agent: Job } = $props();
  // Work log (spec §16.4): while this page is visible and focused, tell the
  // daemon every 30 s so the user's time on the agent is counted.
  const timeId = $derived(agent.id);
  $effect(() => {
    const id = timeId;
    const t = setInterval(() => {
      if (document.visibilityState === 'visible' && document.hasFocus()) post(`/jobs/${id}/user-time`, { ms: 30000 }).catch(() => {});
    }, 30000);
    return () => clearInterval(t);
  });
  let confirmCancel = $state(false);
  let menu = $state(false);
  // Open in… (R-701, R-702, R-706, R-707): links that work because the
  // workspace has the same path on this computer; the browser opens them.
  type Target = { id: string; label: string; kind: 'url'; url?: string };
  let openMenu = $state(false);
  let targets = $state<Target[] | null>(null);
  async function toggleOpen() {
    openMenu = !openMenu;
    if (!openMenu) return;
    try { targets = (await get<{ targets: Target[] }>(`/jobs/${agent.id}/open-targets`)).targets; }
    catch (e) { openMenu = false; showError('Could not list editors', e); }
  }

  const flow = $derived(flowOf(agent, app.state?.flows ?? []));
  // The saved agent this job started from (R23): a link while it exists, plain
  // text after it was deleted; "changed since" opens the diff of its versions.
  const def = $derived(agentOf(agent, app.state?.agents));
  const newer = $derived(changedSince(agent, def));
  const queuedForAgent = $derived(waitsForAgent(agent, app.state?.agents ?? []));
  const tabs = $derived(agentTabs(agent));
  const fallback = $derived.by(() => {
    const g = agent.gate;
    if (g?.kind === 'commit') return 'changes';
    if (prGate(g)) return 'pr';
    if (g?.kind === 'docs') return 'spec';
    return 'terminal';
  });
  const tabId = $derived(tabs.find((t) => t.id === route.query.get('s'))?.id ?? fallback);
  const tab = $derived(tabs.find((t) => t.id === tabId) ?? tabs[0]);
  const Panel = $derived(tab ? PANELS[tab.panel] : undefined);
  const live = $derived(['running', 'waiting-input', 'blocked', 'idle'].includes(agent.status));

  function badge(id: string): { n: string | number; tone: string } | undefined {
    // A gate autonomy will approve at the stop gets no ✋ (R4).
    const g = waitsAtGate(agent) ? agent.gate : undefined;
    if (id === 'spec' && g?.kind === 'docs' && !prGate(g)) return { n: '✋', tone: 'g' };
    if (id === 'pr' && prGate(g)) return { n: '✋', tone: 'g' };
    if (id === 'changes' && g?.kind === 'commit') return { n: '✋', tone: 'g' };
    if (id === 'tasks' && agent.tasks?.length) {
      const blocked = agent.tasks.filter((t) => t.status === 'blocked').length;
      return blocked ? { n: blocked, tone: 'h' } : { n: `${agent.tasks.filter((t) => t.status === 'done').length}/${agent.tasks.length}`, tone: '' };
    }
    if (id === 'commits' && agent.commits?.length) return { n: agent.commits.length, tone: '' };
    if (id === 'memory' && agent.memoryProposal) return { n: '✋', tone: 'g' };
    return undefined;
  }

  const finished = $derived(['done', 'cancelled'].includes(agent.status));
  async function setAutonomy(e: Event) {
    const level = (e.currentTarget as HTMLSelectElement).value;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${agent.id}/autonomy`, { level })).job); }
    catch (err) { showError('Could not change the autonomy', err); }
  }

  async function setMode(e: Event) {
    const mode = (e.currentTarget as HTMLSelectElement).value;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${agent.id}/mode`, { mode })).job); }
    catch (err) { showError('Could not change the tool permissions', err); }
  }

  const canModel = $derived(canChooseModel(app.state?.plugins, agent.runtime ?? 'claude-code'));
  async function setModel(model: string) {
    if (!validModel(model)) { showError('Could not change the model', new Error('Letters, digits and . _ : / @ - [ ] only')); return; }
    if (model.trim() === (agent.model ?? '')) return;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${agent.id}/model`, { model: model.trim() })).job); }
    catch (err) { showError('Could not change the model', err); }
  }

  async function act(action: string, label: string, body?: unknown) {
    menu = false;
    try { upsertJob((await post<{ job: Job }>(`/jobs/${agent.id}/${action}`, body)).job); }
    catch (e) { showError(label, e); }
  }

  // Worktree removal (R12–R14): by hand only, the branch and record stay.
  let removeWithCancel = $state(false);
  let confirmRemove = $state(false);
  const removed = $derived(agent.worktreeRemovedAt != null);
  const canRemove = $derived(finished && !removed && !!agent.workdir);
  function openCancel() { menu = false; removeWithCancel = false; confirmCancel = true; }
</script>

{#if needsYou(agent)}<YourCallBand {agent} {flow} tabId={tab?.id} />{/if}
<TicketPrompt {agent} />
{#if agent.lowered?.length}
  <Callout tone="info">{#each agent.lowered as l (l.field)}<div>
    {l.field === 'autonomy' ? 'Autonomy' : 'Tool permissions'} was lowered from <b>{l.from}</b> to <b>{l.to}</b> by the repository profile <b>{l.profile}</b> ("at most").</div>{/each}</Callout>
{/if}

<header class="hero">
  <div class="htop">
    <div class="hid">
      <div class="row">
        <span class="lbl">{repoName(agent)} · {agent.flow ?? 'no workflow'}</span>
        <TicketPill ticket={agent.ticket} />
      </div>
      <h1 class="hname"><AgentDot {agent} />{agent.name}</h1>
      <div class="hmeta">
        <div class="hfacts">
          {#if agent.agent}
            <span>agent {#if def}<a href={href(`/agents/${def.name}`)}>{agent.agent.name}</a>{:else}{agent.agent.name}{/if} <span class="muted">v{agent.agent.version}</span>
              {#if newer && def}<a class="hint" href={href(`/agents/${def.name}`, { s: 'versions', since: String(agent.agent.version) })} title="This job keeps the settings it started with">· agent changed since this job started (now v{def.version})</a>{/if}
              {#if queuedForAgent}<span class="hint"> · waits for its agent: {def?.liveJobs} of {def?.maxLiveJobs} live jobs</span>{/if}</span>
          {/if}
          {#if agent.base}<span class="mono">{agent.branch} ← {shortBase(agent)}</span>{/if}
          <span>{agent.runtime ?? 'claude-code'}</span>
          {#if !canModel && agent.model}<span class="mono">{agent.model}</span>{/if}
          <span>started {agoText(agent.createdAt)}</span>
          <span>active {agoText(agent.lastActivity)}</span>
        </div>
        <div class="hctl">
          {#if canModel}
            {#key agent.model}
              <label class="hfield" title="Empty = the runtime's default. A change applies from the next session start (approved gate, resume)."><span>model</span>
                <ModelInput inline runtime={agent.runtime ?? 'claude-code'} value={agent.model ?? ''} placeholder="default" disabled={finished} onchange={setModel} /></label>
            {/key}
          {/if}
          {#if agent.flow && agent.flow !== 'none'}
            <label class="hfield"><span>autonomy</span>
              <select aria-label="Autonomy" value={agent.autonomy ?? 'ask'} disabled={finished} onchange={setAutonomy}>
                <option value="ask">ask</option><option value="important">important</option>
              </select></label>
          {/if}
          {#if agent.permissionMode}
            <label class="hfield" title={`${MODE_HINT[agent.permissionMode]} A change applies from the next session start (approved gate, resume).`}><span>tools</span>
              <select aria-label="Tool permissions" value={agent.permissionMode} disabled={finished} onchange={setMode}>
                {#each modes(!!app.state?.sandbox) as m (m)}<option value={m}>{m}</option>{/each}
              </select></label>
          {/if}
        </div>
      </div>
    </div>
    <div class="row">
      <StatusPill {agent} />
      {#if agent.askHold}
        <button class="btn" title="End the session you started with Ask" onclick={() => act('finish', 'Could not finish')}><Icon name="pause" />Finish</button>
      {:else if agent.status === 'paused' || agent.status === 'parked' || agent.status === 'error' || agent.status === 'watching'}
        <button class="btn" onclick={() => act('resume', 'Could not resume')}><Icon name="play" />Resume</button>
      {:else if live && agent.pauseRequestedAt}
        <button class="btn" disabled title="Waiting for the agent to write its handoff note"><Icon name="pause" />Pausing…</button>
      {:else if live}
        <button class="btn" onclick={() => act('pause', 'Could not pause')}><Icon name="pause" />Pause</button>
      {/if}
      {#if agent.workdir && !removed}
        <div class="anchor">
          <button class="btn" aria-haspopup="menu" aria-expanded={openMenu} onclick={toggleOpen}>Open worktree in…</button>
          {#if openMenu}
            <div class="pop" role="menu">
              {#if !targets}<span class="muted mpad">Loading…</span>{/if}
              {#each targets ?? [] as t (t.id)}
                <a role="menuitem" class="btn ghost" href={t.url} onclick={() => (openMenu = false)}>{t.label}</a>
              {/each}
              <a class="hint mpad" href={href('/settings', { tab: 'openin' })} onclick={() => (openMenu = false)}>Manage these…</a>
            </div>
          {/if}
        </div>
      {/if}
      <div class="anchor">
        <button class="btn icon" aria-label="More actions" aria-expanded={menu} onclick={() => (menu = !menu)}><Icon name="more" /></button>
        {#if menu}
          <div class="pop" role="menu">
            <button role="menuitem" class="btn ghost danger" disabled={finished} onclick={openCancel}>Cancel job…</button>
            {#if canRemove}
              <button role="menuitem" class="btn ghost danger" onclick={() => { menu = false; confirmRemove = true; }}>Remove worktree…</button>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
  <Stepper {agent} {flow} />
</header>

<div class="aview">
  <nav class="vnav" aria-label="Job">
    {#each tabs as t (t.id)}
      {@const b = badge(t.id)}
      <a class="vtb" class:on={t.id === tab?.id} aria-current={t.id === tab?.id ? 'page' : undefined}
        href={href(`/jobs/${agent.name}`, { s: t.id })}>
        <Icon name={t.icon} />{t.label}{#if b?.n === '✋'}<span class="tc mark {b.tone}" title="Needs you"><Icon name="handSolid" size={16} solid /></span>{:else if b}<span class="tc {b.tone}">{b.n}</span>{/if}
      </a>
    {/each}
  </nav>
  <div class="apanel">
    {#if removed && tab?.worktree}
      <Callout tone="plain">The worktree was removed {agoText(agent.worktreeRemovedAt ?? 0)}. The branch <span class="mono">{agent.branch}</span> is kept.</Callout>
    {:else if Panel}
      {#key agent.id}<Panel {agent} />{/key}
    {/if}
  </div>
</div>

{#snippet removeWarning()}
  Uncommitted changes and the spec documents in <span class="mono">.xherd/</span> are deleted. The branch <span class="mono">{agent.branch}</span> and the agent's record are kept.
{/snippet}

{#if confirmCancel}
  <Dialog title="Cancel {agent.name}?" onclose={() => (confirmCancel = false)}>
    <p>The agent's session ends and it will not run again. Its branch stays{removeWithCancel ? '' : ', and so does its worktree'}.</p>
    {#if agent.workdir && !removed}
      <label class="f chk"><input type="checkbox" bind:checked={removeWithCancel} /> Also remove the worktree</label>
      {#if removeWithCancel}<p class="hint">{@render removeWarning()}</p>{/if}
    {/if}
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmCancel = false)}>Keep it</button>
      <button class="btn danger" onclick={() => { confirmCancel = false; act('cancel', 'Could not cancel', removeWithCancel ? { removeWorktree: true } : undefined); }}>Cancel agent</button>
    {/snippet}
  </Dialog>
{/if}

{#if confirmRemove}
  <Dialog title="Remove the worktree of {agent.name}?" onclose={() => (confirmRemove = false)}>
    <p>{@render removeWarning()}</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmRemove = false)}>Keep it</button>
      <button class="btn danger" onclick={() => { confirmRemove = false; act('remove-worktree', 'Could not remove the worktree'); }}>Remove worktree</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .anchor { position: relative; }
  .mpad { padding: 6px 10px; }
</style>
