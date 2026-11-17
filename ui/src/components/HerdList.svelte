<script lang="ts">
  // Jobs (the Herd's default view): counters that filter, one list grouped
  // by what you must do (Needs you · Running · Waiting; Done hidden until
  // asked), each job with its stage track, and who in the team is busy.
  import StageTrack from './StageTrack.svelte';
  import JobCounters from './JobCounters.svelte';
  import TicketPill from './TicketPill.svelte';
  import Seg from './Seg.svelte';
  import { href, route, go } from '../lib/router.svelte';
  import { app } from '../lib/store.svelte';
  import { agentLane, agentLaneOrder, jobAgent } from '../lib/agents';
  import { ago, repoName } from '../lib/format';
  import { stageNow } from '../lib/flow';
  import { GROUPS, GROUP_LABEL, jobGroup, initials, avatarBg, filterJobs, isFilter, type JobGroup, type JobFilter } from '../lib/jobs';
  import type { Job, FlowInfo } from '../lib/types';

  let { agents, flows, open }: { agents: Job[]; flows: FlowInfo[]; open: (a: Job) => void } = $props();

  // `?f=` filters by group (all = Done hidden); `?group=` regroups the list.
  const filter = $derived((isFilter(route.query.get('f')) ? route.query.get('f') : 'all') as JobFilter);
  const group = $derived(route.query.get('group') ?? 'status');
  const nav = (o: { f?: string; group?: string }) =>
    go('/', { f: (o.f ?? route.query.get('f')) === 'all' ? undefined : (o.f ?? route.query.get('f') ?? undefined), layout: route.query.get('layout') ?? undefined, group: (o.group ?? group) === 'status' ? undefined : (o.group ?? group) });

  const by = $derived(Object.fromEntries(GROUPS.map((g) => [g, agents.filter((a) => jobGroup(a) === g)])) as Record<JobGroup, Job[]>);
  const shown = $derived(filterJobs(agents, filter));
  const sorted = (l: Job[]) => [...l].sort((a, b) => (jobGroup(a) === 'needs' ? a.statusSince - b.statusSince : b.lastActivity - a.lastActivity));

  interface Block { key: string; label: string; need?: boolean; jobs: Job[] }
  const blocks = $derived.by((): Block[] => {
    if (group === 'agent') {
      const names = agentLaneOrder(shown.map(agentLane));
      return names.map((n) => ({ key: n, label: n, jobs: sorted(shown.filter((j) => agentLane(j) === n)) }));
    }
    if (group === 'flow') {
      const names = [...new Set(shown.map((j) => j.flow ?? 'no workflow'))].sort();
      return names.map((n) => ({ key: n, label: n, jobs: sorted(shown.filter((j) => (j.flow ?? 'no workflow') === n)) }));
    }
    return GROUPS.filter((g) => shown.some((j) => jobGroup(j) === g)).map((g) => ({ key: g, label: GROUP_LABEL[g], need: g === 'needs', jobs: sorted(shown.filter((j) => jobGroup(j) === g)) }));
  });

  const slots = $derived(Number(app.state?.config.maxConcurrentJobs ?? 0));
  const busy = $derived(agents.filter((a) => ['running', 'waiting-input', 'blocked', 'idle'].includes(a.status)).length);
  const team = $derived((app.state?.agents ?? []).filter((a) => a.status !== 'archived'));
  // The live job an agent is on now, and its stage.
  const jobOf = (id: string) => agents.find((j) => j.currentAgent === id && ['running', 'waiting-input', 'blocked', 'idle'].includes(j.status));
  const nowOf = (j: Job) => stageNow(j, flows)?.name ?? '';
</script>

<div class="stack">
  <JobCounters {agents} />

  <div class="row">
    {#if filter !== 'all'}<a class="btn ghost sm" href={href('/')}>Show all</a>{/if}
    <span class="grow"></span>
    <span class="muted">Group</span>
    <Seg label="Group by" value={group} onchange={(g) => nav({ group: g })}
      options={[{ value: 'status', label: 'Status' }, { value: 'agent', label: 'Agent' }, { value: 'flow', label: 'Flow' }]} />
  </div>

  <div class="jcols">
    <section class="card" style="overflow:hidden">
      {#if blocks.length}
        <div class="jrow head"><span>Job</span><span>Stages</span><span>Now</span><span style="text-align:right">Active</span></div>
        {#each blocks as b (b.key)}
          <div class="glabel" class:g={b.need}>{b.label}</div>
          {#each b.jobs as a (a.id)}
            {@const st = stageNow(a, flows)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div class="jrow" class:need={jobGroup(a) === 'needs'} onclick={() => open(a)}>
              <div class="jname">
                <a href={href(`/jobs/${a.name}`)} onclick={(e) => e.stopPropagation()}>{a.name}</a>
                <span class="hint">{#if a.ticket}<TicketPill ticket={a.ticket} />{' · '}{/if}{repoName(a)}{' · '}{a.flow ?? 'no workflow'}</span>
              </div>
              <StageTrack job={a} {flows} />
              <div class="jnow">
                {#if jobGroup(a) === 'needs'}<b style="color:var(--gate)">{a.gate?.kind === 'question' || !a.gate ? (a.status === 'blocked' ? 'Blocked' : 'Your answer') : `${st?.waits ?? 'Decision'} ready`}</b>
                {:else if st}<b>{st.name}</b>{:else}{a.status === 'watching' ? 'Watching the PR' : a.status}{/if}
                {#if jobAgent(a)}{' · '}{jobAgent(a)?.name}{/if}
              </div>
              <span class="num muted" style="text-align:right">{ago(a.lastActivity)}</span>
            </div>
          {/each}
        {/each}
      {:else}
        <div class="cb"><p class="empty">{#if agents.length}Nothing {filter === 'all' ? 'running or waiting' : 'in this list'}.{:else}No jobs yet. <a href={href('/new')}>Start one</a> or run <span class="mono">xherd new</span>.{/if}</p></div>
      {/if}
      {#if filter === 'all' && by.done.length}
        <div class="row" style="padding:10px 16px;border-top:1px solid var(--fence-soft)"><span class="muted">{by.done.length} done</span><span class="grow"></span><a class="btn sm" href={href('/', { f: 'done' })}>Show done</a></div>
      {/if}
    </section>

    <aside class="card" style="overflow:hidden">
      <div class="row" style="padding:12px 14px;border-bottom:1px solid var(--fence-soft)">
        <h2 style="margin:0;font-size:14px">Team now</h2><span class="grow"></span>
        {#if slots}<span class="muted num" style="font-size:12px">{busy} of {slots} slots</span>{/if}
      </div>
      {#each team as t (t.id)}
        {@const j = jobOf(t.id)}
        <div class="trow">
          <span class="av" class:ringg={t.status === 'needs-you'} style:background={avatarBg(t.name)}>{initials(t.name)}</span>
          <div class="t"><b><a href={href(`/agents/${t.name}`)} style="color:var(--ink)">{t.name}</a></b>
            <span>{#if j}{j.name} · {nowOf(j) || j.status}{:else}idle{/if}</span></div>
          <span class="muted num" style="font-size:12px">{t.liveJobs}{#if t.maxLiveJobs}/{t.maxLiveJobs}{/if}</span>
        </div>
      {:else}
        <div class="cb"><p class="empty">No agents yet. <a href={href('/agents/new')}>Create one</a>.</p></div>
      {/each}
      <div class="row" style="padding:10px 14px;border-top:1px solid var(--fence-soft)"><a href={href('/agents')} style="font-size:12px">Open Team</a></div>
    </aside>
  </div>
</div>
