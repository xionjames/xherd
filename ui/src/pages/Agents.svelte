<script lang="ts">
  // Agents (#141, R23): the saved setups jobs start from, with their derived
  // status, live jobs, last job and tokens today. New agent and Start job.
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import { app, reloadAgents } from '../lib/store.svelte';
  import { href, go } from '../lib/router.svelte';
  import { STATUS_LABEL } from '../lib/agents';
  import { agentHue } from '../lib/format';
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

<PageHead title="Agents" sub="A saved setup (runtime, model, autonomy, instructions) that jobs start from.">
  {#snippet actions()}
    <a class="btn primary" href={href('/agents/new')}>New agent</a>
  {/snippet}
</PageHead>

<div class="stack">
  <Panel title="Your agents" flush>
    {#if shown.length}
      <div class="tscroll">
        <table class="tbl">
          <thead><tr><th>Agent</th><th>Status</th><th class="hide-sm">Runtime</th><th class="r">Live jobs</th><th class="hide-sm">Last job</th><th class="hide-sm r">Tokens today</th><th></th></tr></thead>
          <tbody>
            {#each shown as a (a.id)}
              {@render row(a)}
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="cb"><p class="empty">No agents yet. <a href={href('/agents/new')}>Create one</a> or run <span class="mono">xherd agent new</span>.</p></div>
    {/if}
  </Panel>

  {#if archived.length}
    <button class="btn ghost" aria-expanded={showArchived} onclick={() => (showArchived = !showArchived)}>
      {showArchived ? 'Hide' : 'Show'} {archived.length} archived
    </button>
    {#if showArchived}
      <Panel title="Archived" hint="Hidden from pickers; their jobs, memory and versions are kept." flush>
        <div class="tscroll">
          <table class="tbl"><tbody>
            {#each archived as a (a.id)}{@render row(a)}{/each}
          </tbody></table>
        </div>
      </Panel>
    {/if}
  {/if}
</div>

{#snippet row(a: AgentView)}
  <tr class="hov" onclick={() => go(`/agents/${a.name}`)}>
    <td>
      <div class="wrap-any">
        <span class="hue" style:background={agentHue(a.name)}></span>
        <a class="mono nm" href={href(`/agents/${a.name}`)}>{a.name}</a>
        <span class="muted"> v{a.version}</span>
        <div class="hint">{a.description || a.runtime}</div>
        {#if a.broken}<div class="hint err">Cannot start jobs: {a.broken}</div>{/if}
      </div>
    </td>
    <td>
      {#if a.status === 'needs-you'}<span class="pill gate">✋ {STATUS_LABEL[a.status]}</span>
      {:else if a.status === 'working'}<span class="pill live">● {STATUS_LABEL[a.status]}</span>
      {:else}<span class="pill">{STATUS_LABEL[a.status]}</span>{/if}
    </td>
    <td class="hide-sm mono muted">{a.runtime}{a.models?.[a.runtime] ? ` · ${a.models[a.runtime]}` : ''}</td>
    <td class="num r">{a.liveJobs}{#if a.maxLiveJobs}<span class="muted"> / {a.maxLiveJobs}</span>{/if}</td>
    <td class="hide-sm">{#if a.lastJob}<a class="mono" href={href(`/jobs/${a.lastJob}`)} onclick={(e) => e.stopPropagation()}>{a.lastJob}</a>{:else}<span class="muted">–</span>{/if}</td>
    <td class="hide-sm num r">{a.tokensToday ? tokens(a.tokensToday) : '–'}</td>
    <td class="r">
      {#if a.status !== 'archived'}
        <a class="btn" href={href('/new', { agent: a.name })} onclick={(e) => e.stopPropagation()}
          title={a.broken ? `Cannot start jobs: ${a.broken}` : 'Start a job from this agent'} aria-disabled={!!a.broken}>Start job</a>
      {/if}
    </td>
  </tr>
{/snippet}

<style>
  .nm { font-weight: 600; color: var(--ink); }
  .hue { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }
</style>
