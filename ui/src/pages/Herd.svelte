<script lang="ts">
  // Overview: "Needs you" (oldest first) then every agent (spec §17.2), as a
  // list or in columns by stage (R-210, `?layout=stages`).
  import AgentDot from '../components/AgentDot.svelte';
  import NeedsCards from '../components/NeedsCards.svelte';
  import Mini from '../components/Mini.svelte';
  import TaskBar from '../components/TaskBar.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import Seg from '../components/Seg.svelte';
  import Dialog from '../components/Dialog.svelte';
  import StageColumns from '../components/StageColumns.svelte';
  import { app } from '../lib/store.svelte';
  import { href, route, go } from '../lib/router.svelte';
  import { ago, needsYou, repoName, shortBase } from '../lib/format';
  import { post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import type { Agent } from '../lib/types';

  const agents = $derived(app.state?.agents ?? []);
  const waiting = $derived(agents.filter(needsYou).sort((a, b) => a.statusSince - b.statusSince));
  const count = (f: (a: Agent) => boolean) => agents.filter(f).length;
  const slots = $derived(Number(app.state?.config.maxConcurrentAgents ?? 0));
  const busy = $derived(count((a) => ['running', 'waiting-input', 'blocked', 'idle'].includes(a.status)));

  const FILTERS: [string, string, (a: Agent) => boolean][] = [
    ['all', 'All', () => true],
    ['needs', 'Needs you', needsYou],
    ['running', 'Running', (a) => a.status === 'running'],
    ['queued', 'Queued', (a) => a.status === 'queued'],
    ['watching', 'Watching', (a) => a.status === 'watching'],
    ['done', 'Done', (a) => a.status === 'done' || a.status === 'cancelled'],
  ];
  const filter = $derived(route.query.get('f') ?? 'all');
  const layout = $derived(route.query.get('layout') === 'stages' ? 'stages' : 'list');
  const nav = (q: { f?: string; layout?: string }) =>
    go('/', { f: route.query.get('f') ?? undefined, layout: route.query.get('layout') ?? undefined, ...q });
  const shown = $derived(
    agents
      .filter(FILTERS.find((f) => f[0] === filter)?.[2] ?? (() => true))
      .sort((a, b) => b.lastActivity - a.lastActivity),
  );

  // The kill switch (R-032): no handoff notes, nothing new starts.
  const pausable = $derived((app.state?.agents ?? []).filter((a) => ['queued', 'running', 'idle', 'blocked', 'waiting-input'].includes(a.status)).length);
  let confirmPause = $state(false);
  async function pauseAll() {
    confirmPause = false;
    try {
      await post('/agents/pause-all');
    } catch (e) {
      showError('Could not pause the agents', e);
    }
  }
</script>

<PageHead title="Herd">
  {#snippet meta()}
    {#if waiting.length}<span class="pill gate">✋ {waiting.length} need you</span>{/if}
    {#if count((a) => a.status === 'running')}<span class="pill live">● {count((a) => a.status === 'running')} running</span>{/if}
    {#if count((a) => a.status === 'queued')}<span class="pill">{count((a) => a.status === 'queued')} queued</span>{/if}
    {#if count((a) => a.status === 'watching')}<span class="pill done">{count((a) => a.status === 'watching')} watching PR</span>{/if}
    {#if count((a) => a.status === 'done')}<span class="pill">{count((a) => a.status === 'done')} done</span>{/if}
  {/snippet}
  {#snippet actions()}
    {#if slots}<span class="muted num">{busy} of {slots} slots busy</span>{/if}
    {#if pausable}<button class="btn" onclick={() => (confirmPause = true)} title="Kill switch: pause every running and queued agent now">Pause all</button>{/if}
  {/snippet}
</PageHead>

<div class="stack">
  {#if waiting.length}
    <div class="sect"><h2>Needs you</h2><span class="hint">oldest first</span></div>
    <NeedsCards agents={waiting} />
  {/if}

  <Panel title="All agents" flush>
    {#snippet head()}
      <Seg label="Filter" value={filter} onchange={(id) => nav({ f: id === 'all' ? undefined : id })}
        options={FILTERS.map(([id, label, f]) => ({ value: id, label: `${label} ${count(f)}` }))} />
      <Seg label="Layout" value={layout} onchange={(id) => nav({ layout: id === 'stages' ? 'stages' : undefined })}
        options={[{ value: 'list', label: 'List' }, { value: 'stages', label: 'By stage' }]} />
    {/snippet}
    {#if shown.length && layout === 'stages'}
      <StageColumns agents={shown} />
    {:else if shown.length}
      <div class="tscroll">
        <table class="tbl">
          <thead><tr><th>Agent</th><th>Status</th><th class="hide-sm">Progress</th><th class="hide-sm">Tasks</th><th class="hide-sm">Branch</th><th class="r">Active</th></tr></thead>
          <tbody>
            {#each shown as a (a.id)}
              <tr class="hov" onclick={() => go(`/agent/${a.name}`)}>
                <td>
                  <div class="row" style="flex-wrap:nowrap">
                    <AgentDot agent={a} />
                    <div class="wrap-any">
                      <a class="mono nyname" href={href(`/agent/${a.name}`)}>{a.name}</a>
                      <div class="hint">{repoName(a)} · {a.flow ?? 'no workflow'}</div>
                    </div>
                  </div>
                </td>
                <td><StatusPill agent={a} /></td>
                <td class="hide-sm"><Mini agent={a} /></td>
                <td class="hide-sm"><TaskBar agent={a} /></td>
                <td class="hide-sm mono muted">{a.workdir ? `← ${shortBase(a)}` : 'not created'}</td>
                <td class="num muted r">{ago(a.lastActivity)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="cb">
        <p class="empty">{#if agents.length}No agents match this filter.{:else}No agents yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p>
      </div>
    {/if}
  </Panel>
</div>

{#if confirmPause}
  <Dialog title="Pause all {pausable} agents?" onclose={() => (confirmPause = false)}>
    <p>Their sessions end now, without a handoff note, and nothing queued starts. Resume each one when you want it back.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmPause = false)}>Keep them running</button>
      <button class="btn danger" onclick={pauseAll}>Pause all</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .nyname { font-weight: 600; color: var(--ink); }
</style>
