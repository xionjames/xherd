<script lang="ts">
  // Agent page: your-call band, hero (name, meta, actions, progress), tabs of
  // panels from the registry.
  import Icon from '../components/Icon.svelte';
  import AgentDot from '../components/AgentDot.svelte';
  import Stepper from '../components/Stepper.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import YourCallBand from '../components/YourCallBand.svelte';
  import Dialog from '../components/Dialog.svelte';
  import type { Agent } from '../lib/types';
  import { app, upsertAgent } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { PANELS, agentTabs } from '../lib/panels';
  import { flowOf } from '../lib/flow';
  import { agoText, needsYou, repoName, shortBase } from '../lib/format';
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';

  let { agent }: { agent: Agent } = $props();
  let confirmCancel = $state(false);
  let menu = $state(false);
  // Open in… (R-701, R-702, R-704): links that work because the workspace
  // has the same path on this computer, and the host companion's apps.
  type Target = { id: string; label: string; kind: 'url' | 'companion'; url?: string };
  let openMenu = $state(false);
  let targets = $state<Target[] | null>(null);
  async function toggleOpen() {
    openMenu = !openMenu;
    if (!openMenu) return;
    try { targets = (await get<{ targets: Target[] }>(`/agents/${agent.id}/open-targets`)).targets; }
    catch (e) { openMenu = false; showError('Could not list editors', e); }
  }
  async function openWith(t: Target) {
    openMenu = false;
    try { await post(`/agents/${agent.id}/open`, { app: t.id.replace(/^companion:/, '') }); }
    catch (e) { showError(`Could not open in ${t.label}`, e); }
  }

  const flow = $derived(flowOf(agent, app.state?.flows ?? []));
  const tabs = $derived(agentTabs(agent));
  const fallback = $derived.by(() => {
    const g = agent.gate;
    if (g?.kind === 'commit' || (g?.kind === 'docs' && g.role === 'ship')) return 'changes';
    if (g?.kind === 'docs') return 'spec';
    return 'terminal';
  });
  const tabId = $derived(tabs.find((t) => t.id === route.query.get('s'))?.id ?? fallback);
  const tab = $derived(tabs.find((t) => t.id === tabId) ?? tabs[0]);
  const Panel = $derived(tab ? PANELS[tab.panel] : undefined);
  const live = $derived(['running', 'waiting-input', 'blocked', 'idle'].includes(agent.status));

  function badge(id: string): { n: string | number; tone: string } | undefined {
    const g = agent.gate;
    if (id === 'spec' && g?.kind === 'docs' && g.role !== 'ship') return { n: '✋', tone: 'g' };
    if (id === 'changes' && (g?.kind === 'commit' || (g?.kind === 'docs' && g.role === 'ship'))) return { n: '✋', tone: 'g' };
    if (id === 'tasks' && agent.tasks?.length) {
      const blocked = agent.tasks.filter((t) => t.status === 'blocked').length;
      return blocked ? { n: blocked, tone: 'h' } : { n: `${agent.tasks.filter((t) => t.status === 'done').length}/${agent.tasks.length}`, tone: '' };
    }
    if (id === 'commits' && agent.commits?.length) return { n: agent.commits.length, tone: '' };
    return undefined;
  }

  async function act(action: string, label: string) {
    menu = false;
    try { upsertAgent((await post<{ agent: Agent }>(`/agents/${agent.id}/${action}`)).agent); }
    catch (e) { showError(label, e); }
  }
</script>

{#if needsYou(agent)}<YourCallBand {agent} {flow} tabId={tab?.id} />{/if}

<header class="hero">
  <div class="htop">
    <div style="display:flex;flex-direction:column;gap:6px;min-width:0;flex:1">
      <div class="row lbl" style="gap:10px">
        <span>{repoName(agent)} · {agent.flow ?? 'no workflow'}</span>
        <span class="pill acc mono" style="text-transform:none;letter-spacing:0">{agent.ticketKey}</span>
      </div>
      <h1 class="hname"><AgentDot {agent} />{agent.name}</h1>
      <div class="hmeta">
        {#if agent.base}<span class="mono">{agent.name} ← {shortBase(agent)}</span>{/if}
        <span>{agent.runtime ?? 'claude-code'}</span>
        <span>started {agoText(agent.createdAt)}</span>
        <span>active {agoText(agent.lastActivity)}</span>
      </div>
    </div>
    <div class="row">
      <StatusPill {agent} />
      {#if agent.status === 'paused' || agent.status === 'parked' || agent.status === 'error'}
        <button class="btn" onclick={() => act('resume', 'Could not resume')}><Icon name="play" />Resume</button>
      {:else if live}
        <button class="btn" onclick={() => act('pause', 'Could not pause')}><Icon name="pause" />Pause</button>
      {/if}
      {#if agent.workdir}
        <div style="position:relative">
          <button class="btn" aria-haspopup="menu" aria-expanded={openMenu} onclick={toggleOpen}>Open in…</button>
          {#if openMenu}
            <div class="menu card" role="menu">
              {#if !targets}<span class="muted" style="padding:6px 10px">Loading…</span>{/if}
              {#each targets ?? [] as t (t.id)}
                {#if t.kind === 'url'}
                  <a role="menuitem" class="btn ghost" href={t.url} onclick={() => (openMenu = false)}>{t.label}</a>
                {:else}
                  <button role="menuitem" class="btn ghost" onclick={() => openWith(t)}>{t.label}</button>
                {/if}
              {/each}
              {#if targets && !targets.some((t) => t.kind === 'companion')}
                <span class="hint" style="padding:6px 10px">Run <span class="mono">xherd companion</span> on this computer to offer its apps here.</span>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
      <div style="position:relative">
        <button class="btn icon" aria-label="More actions" aria-expanded={menu} onclick={() => (menu = !menu)}><Icon name="more" /></button>
        {#if menu}
          <div class="menu card" role="menu">
            <button role="menuitem" class="btn ghost danger" disabled={agent.status === 'cancelled' || agent.status === 'done'}
              onclick={() => { menu = false; confirmCancel = true; }}>Cancel agent…</button>
          </div>
        {/if}
      </div>
    </div>
  </div>
  <Stepper {agent} {flow} />
  <nav class="tabs" aria-label="Agent">
    {#each tabs as t (t.id)}
      {@const b = badge(t.id)}
      <a class="tb" class:on={t.id === tab?.id} aria-current={t.id === tab?.id ? 'page' : undefined}
        href={href(`/agent/${agent.name}`, { s: t.id })}>
        <Icon name={t.icon} />{t.label}{#if b}<span class="tc {b.tone}">{b.n}</span>{/if}
      </a>
    {/each}
  </nav>
</header>

{#if Panel}
  {#key agent.id}<Panel {agent} />{/key}
{/if}

{#if confirmCancel}
  <Dialog title="Cancel {agent.name}?" onclose={() => (confirmCancel = false)}>
    <p>The agent's session ends and it will not run again. Its worktree and branch stay.</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (confirmCancel = false)}>Keep it</button>
      <button class="btn dfill" onclick={() => { confirmCancel = false; act('cancel', 'Could not cancel'); }}>Cancel agent</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .menu { position: absolute; right: 0; top: 34px; z-index: 30; padding: 4px; min-width: 180px; display: flex; flex-direction: column; }
  .menu .btn { justify-content: flex-start; }
</style>
