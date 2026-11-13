<script lang="ts">
  // Agent page: your-call band, hero (name, meta, actions, progress), tabs of
  // panels from the registry.
  import Icon from '../components/Icon.svelte';
  import { linkFor } from '../lib/links';
  import AgentDot from '../components/AgentDot.svelte';
  import Stepper from '../components/Stepper.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import YourCallBand from '../components/YourCallBand.svelte';
  import Dialog from '../components/Dialog.svelte';
  import Callout from '../components/Callout.svelte';
  import TicketPrompt from '../components/TicketPrompt.svelte';
  import type { Agent } from '../lib/types';
  import { app, upsertAgent } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { PANELS, agentTabs } from '../lib/panels';
  import { flowOf, prGate } from '../lib/flow';
  import { agoText, needsYou, repoName, shortBase } from '../lib/format';
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { MODE_HINT, modes } from '../lib/modes';

  let { agent }: { agent: Agent } = $props();
  // Work log (spec §16.4): while this page is visible and focused, tell the
  // daemon every 30 s so the user's time on the agent is counted.
  const timeId = $derived(agent.id);
  $effect(() => {
    const id = timeId;
    const t = setInterval(() => {
      if (document.visibilityState === 'visible' && document.hasFocus()) post(`/agents/${id}/user-time`, { ms: 30000 }).catch(() => {});
    }, 30000);
    return () => clearInterval(t);
  });
  let confirmCancel = $state(false);
  let menu = $state(false);
  // Open in… (R-701, R-702, R-704, R-706, R-707): links that work because the
  // workspace has the same path on this computer, and what the host companion
  // runs: the terminal, its apps and your commands from Settings → Open in.
  type Target = { id: string; label: string; kind: 'url' | 'companion'; url?: string };
  let openMenu = $state(false);
  let targets = $state<Target[] | null>(null);
  async function toggleOpen() {
    openMenu = !openMenu;
    if (!openMenu) return;
    try { targets = (await get<{ targets: Target[] }>(`/agents/${agent.id}/open-targets`)).targets; }
    catch (e) { openMenu = false; showError('Could not list editors', e); }
  }
  // A command of yours runs only once confirmed in the companion's window.
  let sentCmd = $state<string | null>(null);
  async function openWith(t: Target) {
    openMenu = false;
    try {
      await post(`/agents/${agent.id}/open`, { app: t.id });
      sentCmd = t.id.startsWith('cmd:') ? t.label : null;
    } catch (e) { showError(`Could not open in ${t.label}`, e); }
  }

  const flow = $derived(flowOf(agent, app.state?.flows ?? []));
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
    const g = agent.gate;
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
    try { upsertAgent((await post<{ agent: Agent }>(`/agents/${agent.id}/autonomy`, { level })).agent); }
    catch (err) { showError('Could not change the autonomy', err); }
  }

  async function setMode(e: Event) {
    const mode = (e.currentTarget as HTMLSelectElement).value;
    try { upsertAgent((await post<{ agent: Agent }>(`/agents/${agent.id}/mode`, { mode })).agent); }
    catch (err) { showError('Could not change the tool permissions', err); }
  }

  async function act(action: string, label: string) {
    menu = false;
    try { upsertAgent((await post<{ agent: Agent }>(`/agents/${agent.id}/${action}`)).agent); }
    catch (e) { showError(label, e); }
  }
</script>

{#if needsYou(agent)}<YourCallBand {agent} {flow} tabId={tab?.id} />{/if}
<TicketPrompt {agent} />

<header class="hero">
  <div class="htop">
    <div class="hid">
      <div class="row">
        <span class="lbl">{repoName(agent)} · {agent.flow ?? 'no workflow'}</span>
        {#if linkFor(agent.ticketKey)}<a class="pill acc mono" href={linkFor(agent.ticketKey)} target="_blank" rel="noopener noreferrer" title="Open the ticket">{agent.ticketKey}</a>{:else}<span class="pill acc mono">{agent.ticketKey}</span>{/if}
      </div>
      <h1 class="hname"><AgentDot {agent} />{agent.name}</h1>
      <div class="hmeta">
        {#if agent.base}<span class="mono">{agent.name} ← {shortBase(agent)}</span>{/if}
        <span>{agent.runtime ?? 'claude-code'}</span>
        {#if agent.flow && agent.flow !== 'none'}
          <label class="row" style="gap:6px;flex-wrap:nowrap">autonomy
            <select aria-label="Autonomy" value={agent.autonomy ?? 'ask'} disabled={finished} onchange={setAutonomy}>
              <option value="ask">ask</option><option value="important">important</option>
            </select></label>
        {/if}
        {#if agent.permissionMode}
          <label class="row" style="gap:6px;flex-wrap:nowrap" title={`${MODE_HINT[agent.permissionMode]} A change applies from the next session start (approved gate, resume).`}>tools
            <select aria-label="Tool permissions" value={agent.permissionMode} disabled={finished} onchange={setMode}>
              {#each modes(!!app.state?.sandbox) as m (m)}<option value={m}>{m}</option>{/each}
            </select></label>
        {/if}
        <span>started {agoText(agent.createdAt)}</span>
        <span>active {agoText(agent.lastActivity)}</span>
      </div>
    </div>
    <div class="row">
      <StatusPill {agent} />
      {#if agent.status === 'paused' || agent.status === 'parked' || agent.status === 'error' || agent.status === 'watching'}
        <button class="btn" onclick={() => act('resume', 'Could not resume')}><Icon name="play" />Resume</button>
      {:else if live && agent.pauseRequestedAt}
        <button class="btn" disabled title="Waiting for the agent to write its handoff note"><Icon name="pause" />Pausing…</button>
      {:else if live}
        <button class="btn" onclick={() => act('pause', 'Could not pause')}><Icon name="pause" />Pause</button>
      {/if}
      {#if agent.workdir}
        <div class="anchor">
          <button class="btn" aria-haspopup="menu" aria-expanded={openMenu} onclick={toggleOpen}>Open in…</button>
          {#if openMenu}
            <div class="pop" role="menu">
              {#if !targets}<span class="muted mpad">Loading…</span>{/if}
              {#each targets ?? [] as t (t.id)}
                {#if t.kind === 'url'}
                  <a role="menuitem" class="btn ghost" href={t.url} onclick={() => (openMenu = false)}>{t.label}</a>
                {:else}
                  <button role="menuitem" class="btn ghost" onclick={() => openWith(t)}>{t.label}</button>
                {/if}
              {/each}
              {#if targets && !targets.some((t) => t.kind === 'companion')}
                <span class="hint mpad">Run <span class="mono">xherd companion</span> on this computer for the terminal and its apps.</span>
              {/if}
              <a class="hint mpad" href={href('/settings', { tab: 'openin' })} onclick={() => (openMenu = false)}>Manage these…</a>
            </div>
          {/if}
        </div>
      {/if}
      <div class="anchor">
        <button class="btn icon" aria-label="More actions" aria-expanded={menu} onclick={() => (menu = !menu)}><Icon name="more" /></button>
        {#if menu}
          <div class="pop" role="menu">
            <button role="menuitem" class="btn ghost danger" disabled={agent.status === 'cancelled' || agent.status === 'done'}
              onclick={() => { menu = false; confirmCancel = true; }}>Cancel agent…</button>
          </div>
        {/if}
      </div>
    </div>
  </div>
  <Stepper {agent} {flow} />
</header>
{#if sentCmd}
  <Callout tone="info">Sent to the companion: {sentCmd}. If it is new or changed, confirm it in the companion's window.
    <button class="btn ghost sm" onclick={() => (sentCmd = null)}>OK</button></Callout>
{/if}

<div class="aview">
  <nav class="vnav" aria-label="Agent">
    {#each tabs as t (t.id)}
      {@const b = badge(t.id)}
      <a class="vtb" class:on={t.id === tab?.id} aria-current={t.id === tab?.id ? 'page' : undefined}
        href={href(`/agent/${agent.name}`, { s: t.id })}>
        <Icon name={t.icon} />{t.label}{#if b?.n === '✋'}<span class="tc mark {b.tone}" title="Needs you"><Icon name="handSolid" size={16} solid /></span>{:else if b}<span class="tc {b.tone}">{b.n}</span>{/if}
      </a>
    {/each}
  </nav>
  <div class="apanel">
    {#if Panel}
      {#key agent.id}<Panel {agent} />{/key}
    {/if}
  </div>
</div>

{#if confirmCancel}
  <Dialog title="Cancel {agent.name}?" onclose={() => (confirmCancel = false)}>
    <p>The agent's session ends and it will not run again. Its worktree and branch stay.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmCancel = false)}>Keep it</button>
      <button class="btn danger" onclick={() => { confirmCancel = false; act('cancel', 'Could not cancel'); }}>Cancel agent</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .anchor { position: relative; }
  .mpad { padding: 6px 10px; }
</style>
