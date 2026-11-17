<script lang="ts">
  // Agents (#141, R23): the saved setups jobs start from, with their derived
  // status, live jobs, last job and tokens today. New agent and Start job.
  import PageHead from '../components/PageHead.svelte';
  import TeamTabs from '../components/TeamTabs.svelte';
  import { app, reloadAgents } from '../lib/store.svelte';
  import { href } from '../lib/router.svelte';
  import { STATUS_LABEL } from '../lib/agents';
  import { initials, avatarBg } from '../lib/jobs';
  import type { AgentView } from '../lib/types';

  const all = $derived(app.state?.agents ?? []);
  const archived = $derived(all.filter((a) => a.status === 'archived'));
  const shown = $derived(all.filter((a) => a.status !== 'archived'));
  let showArchived = $state(false);

  // Status, live jobs and the last job come from the jobs: fetch again when
  // a job of an agent changes state (agent edits arrive as a change message).
  const signature = $derived((app.state?.jobs ?? []).map((j) => `${j.id}:${j.status}:${j.currentAgent ?? ''}:${Object.keys(j.agents ?? {}).join(',')}`).join('|'));
  $effect(() => {
    signature;
    reloadAgents();
  });

  const tokens = (n: number) => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));
</script>

<PageHead title="Team" sub="Agents are saved setups (runtime, model, autonomy, instructions) that jobs and workflow stages draw from.">
  {#snippet actions()}
    <a class="btn primary" href={href('/agents/new')}>New agent</a>
  {/snippet}
  {#snippet tabs()}<TeamTabs current="agents" />{/snippet}
</PageHead>

<div class="stack">
  {#if shown.length}
    <div class="acards">
      {#each shown as a (a.id)}{@render card(a)}{/each}
    </div>
  {:else}
    <p class="empty">No agents yet. <a href={href('/agents/new')}>Create one</a> or run <span class="mono">xherd agent new</span>.</p>
  {/if}

  {#if archived.length}
    <div class="row">
      <button class="btn ghost" aria-expanded={showArchived} onclick={() => (showArchived = !showArchived)}>
        {showArchived ? 'Hide' : 'Show'} {archived.length} archived
      </button>
      <span class="hint">Hidden from pickers; their jobs, memory and versions are kept.</span>
    </div>
    {#if showArchived}
      <div class="acards">
        {#each archived as a (a.id)}{@render card(a)}{/each}
      </div>
    {/if}
  {/if}
</div>

{#snippet card(a: AgentView)}
  <article class="acard">
    <div class="h">
      <span class="av l" class:ringg={a.status === 'needs-you'} class:ring={a.status === 'working'} style:background={avatarBg(a.name)}>{initials(a.name)}</span>
      <div class="t">
        <a class="mono nm" href={href(`/agents/${a.name}`)}>{a.name}</a>
        <span class="hint">v{a.version} · {a.description || a.runtime}</span>
      </div>
      <span class="grow"></span>
      {#if a.status === 'needs-you'}<span class="pill gate">✋ {STATUS_LABEL[a.status]}</span>
      {:else if a.status === 'working'}<span class="pill live">● {STATUS_LABEL[a.status]}</span>
      {:else}<span class="pill">{STATUS_LABEL[a.status]}</span>{/if}
    </div>
    <div class="b">
      <dl class="kv2">
        <dt>Runs on</dt><dd class="mono">{a.runtime}{a.models?.[a.runtime] ? ` · ${a.models[a.runtime]}` : ''}</dd>
        <dt>Jobs</dt><dd>{a.liveJobs}{#if a.maxLiveJobs}{' of '}{a.maxLiveJobs}{/if}{' live'}{#if a.lastJob}{' · last '}<a class="mono" href={href(`/jobs/${a.lastJob}`)}>{a.lastJob}</a>{/if}</dd>
        <dt>Tokens today</dt><dd>{a.tokensToday ? tokens(a.tokensToday) : '–'}</dd>
      </dl>
      {#if a.broken}<span class="hint err">Cannot start jobs: {a.broken}</span>{/if}
    </div>
    <div class="ft">
      <a class="btn sm" href={href(`/agents/${a.name}`)}>Open</a>
      <span class="grow"></span>
      {#if a.status !== 'archived'}
        <a class="btn sm" href={href('/new', { agent: a.name })} title={a.broken ? `Cannot start jobs: ${a.broken}` : 'Start a job from this agent'} aria-disabled={!!a.broken}>Start job</a>
      {/if}
    </div>
  </article>
{/snippet}

<style>
  .nm { font-weight: 600; color: var(--ink); text-decoration: none; }
</style>
