<script lang="ts">
  // Inbox: what stops a job (coral: gates, questions, blocks, due TODOs) and,
  // as news, what xherd approved by itself (R-111, R2; "Mark seen" hides an
  // item, the ledger keeps it). Pick an item on the left, see it and go to its
  // decision on the right. Approvals never count as needing you (R1).
  import { app } from '../lib/store.svelte';
  import { post } from '../lib/api';
  import { href, route, go } from '../lib/router.svelte';
  import { agoText, ago, waitText } from '../lib/format';
  import { showError } from '../lib/errors.svelte';
  import { attention, refresh, waitingAgents, dueTodos, unseenApprovals, approvalHref, type AutoApproval } from '../lib/attention.svelte';
  import { decisionTab, actionLabel, waitTitle, waitMeta } from '../lib/inbox';
  import { stageNow } from '../lib/flow';
  import { agentFor } from '../lib/agents';
  import { initials, avatarBg } from '../lib/jobs';
  import PageHead from '../components/PageHead.svelte';
  import Seg from '../components/Seg.svelte';
  import StageTrack from '../components/StageTrack.svelte';
  import type { Job } from '../lib/types';

  $effect(() => { refresh(); });

  const waiting = $derived(waitingAgents());
  const todos = $derived(dueTodos());
  const unseen = $derived(unseenApprovals());
  const flows = $derived(app.state?.flows ?? []);
  const name = (id: string) => app.state?.jobs.find((a) => a.id === id)?.name;
  const key = (i: AutoApproval) => `a:${i.jobId}:${i.at}:${i.gate}`;

  // `?show=` all | needs | updates; `?item=` the open one (j:<job>, t:<todo>, a:…).
  const show = $derived(route.query.get('show') ?? 'all');
  const setShow = (v: string) => go('/inbox', { show: v === 'all' ? undefined : v });
  const pick = (id: string) => go('/inbox', { show: show === 'all' ? undefined : show, item: id });
  const updates = $derived((attention.reviews ?? []).filter((i) => !i.seen));
  const needsCount = $derived(waiting.length + todos.length);

  const items = $derived.by(() => [
    ...(show !== 'updates' ? waiting.map((j) => ({ id: `j:${j.name}`, kind: 'job' as const, job: j })) : []),
    ...(show !== 'updates' ? todos.map((t) => ({ id: `t:${t.id}`, kind: 'todo' as const, todo: t })) : []),
    ...(show !== 'needs' ? updates.map((u) => ({ id: key(u), kind: 'approval' as const, ap: u })) : []),
  ]);
  const sel = $derived(items.find((i) => i.id === route.query.get('item')) ?? items[0]);

  async function markSeen(list: AutoApproval[]) {
    try {
      await post('/reviews/seen', { items: list.map(({ jobId, at, gate }) => ({ jobId, at, gate })) });
      await refresh();
    } catch (e) { showError('Could not mark them seen', e); }
  }
  const who = (j: Job) => agentFor(j, j.stage) ?? undefined;
</script>

<PageHead title="Inbox">
  {#snippet sub()}Only the coral items stop a job. The rest is news.{/snippet}
  {#snippet actions()}
    <Seg label="Show" value={show} onchange={setShow}
      options={[{ value: 'all', label: 'All' }, { value: 'needs', label: `Needs you ${needsCount}` }, { value: 'updates', label: `Updates ${updates.length}` }]} />
    {#if unseen.length}<button class="btn sm" onclick={() => markSeen(unseen)}>Mark all read</button>{/if}
  {/snippet}
</PageHead>

<div class="stack">
  {#if !items.length}
    <p class="empty">{needsCount || updates.length ? 'Nothing here.' : 'Nothing needs you right now.'}</p>
  {:else}
    <div class="jcols inbox">
      <section class="card ibxl" style="overflow:hidden" aria-label="Inbox items">
        {#if waiting.length + todos.length && show !== 'updates'}<div class="glabel g">Needs you · {needsCount}</div>{/if}
        {#each items.filter((i) => i.kind !== 'approval') as it (it.id)}
          {#if it.kind === 'job'}
            <a class="item need" class:on={sel?.id === it.id} href={href('/inbox', { show: show === 'all' ? undefined : show, item: it.id })}>
              <span class="av ringg" style:background={avatarBg(who(it.job))}>{initials(who(it.job))}</span>
              <div class="t"><b>{waitTitle(it.job)} · {it.job.name}</b><span>{waitText(it.job)}</span></div>
              <span class="muted num" style="font-size:12px">{ago(it.job.statusSince)}</span>
            </a>
          {:else if it.kind === 'todo'}
            <a class="item need" class:on={sel?.id === it.id} href={href('/inbox', { show: show === 'all' ? undefined : show, item: it.id })}>
              <span class="av ringg">✓</span>
              <div class="t"><b>{it.todo.title}</b><span>{it.todo.status === 'error' ? 'TODO check failed' : 'TODO is due'}</span></div>
              <span class="muted num" style="font-size:12px">{ago(it.todo.due)}</span>
            </a>
          {/if}
        {/each}
        {#if updates.length && show !== 'needs'}<div class="glabel">Updates · {updates.length}</div>{/if}
        {#each items.filter((i) => i.kind === 'approval') as it (it.id)}
          {#if it.kind === 'approval'}
            <a class="item" class:on={sel?.id === it.id} href={href('/inbox', { show: show === 'all' ? undefined : show, item: it.id })}>
              <span class="av">{initials(name(it.ap.jobId))}</span>
              <div class="t"><b>xherd approved {it.ap.gate}</b><span>{name(it.ap.jobId) ?? 'removed job'}{it.ap.reason ? ` · ${it.ap.reason}` : ''}</span></div>
              <span class="muted num" style="font-size:12px">{ago(it.ap.at)}</span>
            </a>
          {/if}
        {/each}
      </section>

      <section class="card" style="overflow:hidden">
        {#if sel?.kind === 'job'}
          {@const j = sel.job}
          <div class="row" style="padding:14px 16px;gap:12px;border-bottom:1px solid var(--fence-soft)">
            <span class="av ringg" style:background={avatarBg(who(j))}>{initials(who(j))}</span>
            <div style="display:flex;flex-direction:column;min-width:0">
              <h2 style="margin:0;font-size:15px">{waitTitle(j)}</h2>
              <span class="muted" style="font-size:12px"><a class="mono" href={href(`/jobs/${j.name}`)}>{j.name}</a>{#if j.flow}{' · '}{j.flow}{/if}{#if who(j)}{' · '}{who(j)}{/if}{' · '}{agoText(j.statusSince)}</span>
            </div>
            <span class="grow"></span>
            <a class="btn sm" href={href(`/jobs/${j.name}`)}>Open job</a>
          </div>
          <div class="sum">
            <div class="row"><StageTrack job={j} {flows} />{#if stageNow(j, flows)}<span class="muted" style="font-size:12px">at {stageNow(j, flows)?.name}</span>{/if}</div>
            <p class="wrap-any" style="margin:0" class:mono={j.gate?.kind === 'commit'}>{waitText(j)}</p>
            <p class="hint" style="margin:0">{waitMeta(j)}</p>
          </div>
          <div class="dec"><button class="btn gate" onclick={() => go(`/jobs/${j.name}`, { s: decisionTab(j) })}>{actionLabel(j)}</button><span class="hint">Opens the job on the tab that holds the decision.</span></div>
        {:else if sel?.kind === 'todo'}
          {@const t = sel.todo}
          <div class="sum">
            <h3>{t.title}</h3>
            <p class="muted" style="margin:0">{t.status === 'error' ? 'The condition check failed.' : 'This TODO is due.'} {agoText(t.due)}</p>
          </div>
          <div class="dec"><a class="btn gate" href={href('/todos')}>Open TODOs</a></div>
        {:else if sel?.kind === 'approval'}
          {@const it = sel.ap}
          <div class="sum">
            <h3>xherd approved {it.gate}{#if name(it.jobId)} for <a class="mono" href={href(`/jobs/${name(it.jobId)}`)}>{name(it.jobId)}</a>{/if}</h3>
            <p class="muted" style="margin:0">{agoText(it.at)}. Under autonomy important; anything a tripwire held waited for you instead.</p>
            {#if it.reason}<p style="margin:0">{it.reason}</p>{/if}
            {#if it.files?.length}<p class="mono hint wrap-any" style="margin:0">{it.files.join(', ')}</p>{/if}
          </div>
          <div class="dec">
            {#if name(it.jobId)}<a class="btn" href={approvalHref(it, name(it.jobId)!)}>Open what was approved</a>{/if}
            <span class="grow"></span>
            <button class="btn" onclick={() => markSeen([it])}>Mark seen</button>
          </div>
        {/if}
      </section>
    </div>
  {/if}
</div>
