<script lang="ts">
  // Flows (D26, R35, R42): which saved agent runs each stage of a workflow, and
  // whether that stage's gate always waits for you. `#/flows` lists the
  // workflows, `#/flows/<pack>/<flow>` edits one. A stage with no agent of its
  // own runs the previous stage's (or the job's, when it starts from one).
  import PageHead from '../components/PageHead.svelte';
  import TeamTabs from '../components/TeamTabs.svelte';
  import Panel from '../components/Panel.svelte';
  import Seg from '../components/Seg.svelte';
  import SaveBar from '../components/SaveBar.svelte';
  import Callout from '../components/Callout.svelte';
  import { app } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { get, put } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { stageLabel } from '../lib/flow';
  import { assignmentRows, pickable } from '../lib/agents';
  import { initials, avatarBg } from '../lib/jobs';
  import type { FlowStageView, FlowView, AssignmentRow } from '../lib/types';

  const m = $derived(route.path.match(/^\/flows\/([^/]+)\/([^/]+)$/));
  const list = $derived(app.state?.flows ?? []);
  // `#/flows` shows the first workflow; the list on the left picks another.
  const pack = $derived(m?.[1] ? decodeURIComponent(m[1]) : (list[0]?.pack ?? ''));
  const flowName = $derived(m?.[2] ? decodeURIComponent(m[2]) : (list[0]?.flow ?? ''));
  const jobsOf = (f: string) => (app.state?.jobs ?? []).filter((j) => j.flow === f).length;
  let sel = $state(0);
  $effect(() => { pack; flowName; sel = 0; });
  const agents = $derived((app.state?.agents ?? []));

  type Row = { agent: string; human: boolean; fresh: boolean };
  let view = $state<FlowView | null>(null);
  let rows = $state<Row[]>([]);
  let orig = $state<Row[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const label = (s: FlowStageView) => stageLabel({ ...s, gate: false });
  const toRows = (v: FlowView): Row[] => v.stages.map((s) => ({ agent: s.assigned ? (s.agent ?? '') : '', human: s.human, fresh: s.freshCheck }));

  async function load(p: string, f: string) {
    view = null; error = null;
    try {
      const v = await get<FlowView>(`/flows/${encodeURIComponent(p)}/${encodeURIComponent(f)}`);
      view = v; rows = toRows(v); orig = toRows(v);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => { if (pack && flowName) load(pack, flowName); });
  // Agents change (archived, runtime switched off): the page reads them again.
  const sig = $derived(agents.map((a) => `${a.id}:${a.version}:${a.status}:${a.broken ?? ''}`).join('|'));
  $effect(() => { sig; if (pack && flowName && !dirty && !busy) load(pack, flowName); });

  const dirty = $derived(rows.reduce((n, r, i) => n + (r.agent !== orig[i]?.agent || r.human !== orig[i]?.human || r.fresh !== orig[i]?.fresh ? 1 : 0), 0));
  const choices = $derived(agents.filter(pickable));
  // A stage whose agent is archived or gone is still listed, so it can be changed.
  const names = (cur: string) => (cur && !choices.some((a) => a.name === cur) ? [...choices.map((a) => a.name), cur] : choices.map((a) => a.name));
  const withDesc = (n: string) => { const d = agents.find((a) => a.name === n)?.description?.trim(); return d ? `${n} - ${d}` : n; };

  async function save() {
    if (!view) return;
    busy = true;
    try {
      const body: { assignments: AssignmentRow[]; freshCheck: string[] } = {
        assignments: assignmentRows(rows.map((r, i) => ({ stage: view!.stages[i].id, agent: r.agent, human: r.human })), agents),
        freshCheck: view.stages.filter((_, i) => rows[i].fresh).map((s) => s.id),
      };
      const v = await put<FlowView>(`/flows/${encodeURIComponent(pack)}/${encodeURIComponent(flowName)}/assignments`, body);
      view = v; rows = toRows(v); orig = toRows(v);
    } catch (e) {
      showError('The assignments were not saved', e);
    } finally {
      busy = false;
    }
  }
  const discard = () => { rows = orig.map((r) => ({ ...r })); };
  // A stage's agent as it would resolve with the edits: its own, else the one before's.
  const resolved = $derived.by(() => {
    let prev = '';
    return rows.map((r) => (prev = r.agent || prev));
  });
</script>

<PageHead title="Team" sub="Give each stage of a workflow to a saved agent.">
  {#snippet tabs()}<TeamTabs current="flows" />{/snippet}
</PageHead>
<div class="stack">
  {#if !list.length}
    <p class="empty">No workflows.</p>
  {:else}
    <div class="flw">
      <nav class="flw-list" aria-label="Workflows">
        {#each list as f (f.pack + '/' + f.flow)}
          {@const n = jobsOf(f.flow)}
          <a class="flw-item" class:on={f.pack === pack && f.flow === flowName} href={href(`/flows/${f.pack}/${f.flow}`)}>
            <b class="mono">{f.flow}</b>
            <span class="hint">{f.stages.length} stages · {n ? `${n} job${n === 1 ? '' : 's'}` : 'not used'}</span>
          </a>
        {/each}
      </nav>
      <div class="flw-main">
        {#if error}
          <Callout tone="bad">{error}</Callout>
        {:else if !view}
          <p class="empty">Loading…</p>
        {:else}
          {@const st = view.stages[sel] ?? view.stages[0]}
          {@const i = view.stages.indexOf(st)}
          <div class="flw-head"><b class="mono big">{flowName}</b> <span class="hint">from pack {pack}</span></div>
          <div class="flw-track">
            {#each view.stages as s, k (s.id)}
              {@const who = resolved[k]}
              {@const always = rows[k].human || s.class === 'human-only'}
              <button class="flw-st" class:on={k === i} class:inh={!rows[k].agent} onclick={() => (sel = k)} aria-pressed={k === i}>
                <b>{label(s)}</b>
                <span class="flw-ag"><span class="av s" style:background={avatarBg(who)}>{initials(who)}</span>{who || 'no agent'}</span>
                <span class="hint" class:flw-hand={always}>{always ? '✋ always' : 'by autonomy'}</span>
              </button>
              {#if k < view.stages.length - 1}<span class="flw-arr" aria-hidden="true">›</span>{/if}
            {/each}
          </div>
          <p class="hint">Solid = agent set here · Dashed = same as the stage before · ✋ = the gate always waits for you</p>
          <Panel title="Stage · {label(st)}" hint="{i + 1} of {view.stages.length} · {st.id}">
            <div class="grp">
              <label class="f">Agent
                <span class="row">
                  <select aria-label="Agent for {label(st)}" bind:value={rows[i].agent}>
                    <option value="">{resolved[i] && !rows[i].agent ? `Same as before (${resolved[i]})` : 'No agent'}</option>
                    {#each names(rows[i].agent) as n (n)}<option value={n}>{withDesc(n)}</option>{/each}
                  </select>
                  {#if rows[i].agent}<button class="btn sm" onclick={() => (rows[i].agent = '')}>Inherit instead</button>{/if}
                </span>
                {#if st.broken && rows[i].agent === (st.agent ?? '') && st.assigned}<span class="hint err">✕ Cannot start jobs: {st.broken}</span>{/if}
              </label>
              <div class="f">When it finishes
                <Seg label="When the stage finishes" value={rows[i].human ? 'wait' : 'auto'} onchange={(v) => (rows[i].human = v === 'wait')}
                  options={[{ value: 'auto', label: 'Follow autonomy', disabled: !rows[i].agent || st.class === 'human-only' }, { value: 'wait', label: 'Always wait for me', disabled: !rows[i].agent }]} />
                <span class="hint">{st.class === 'human-only' ? 'This stage always waits for you.' : '“Always wait” cannot be skipped, even at higher autonomy.'}</span>
              </div>
              {#if st.freshCheckAllowed}
                <div class="f">Fresh-context check
                  <Seg label="Fresh-context check for {label(st)}" value={rows[i].fresh ? 'on' : 'off'} onchange={(v) => (rows[i].fresh = v === 'on')}
                    options={[{ value: 'off', label: 'Off' }, { value: 'on', label: 'On' }]} />
                  <span class="hint">Before this stage asks for your approval, a helper that has not seen the conversation reads the documents and says what it understood, then points out gaps or unclear parts. The agent fixes what it can and leaves the rest as questions for you. It runs once each time the gate opens and uses a little extra time and tokens. If the runtime has no helper, it is skipped.</span>
                </div>
              {/if}
            </div>
          </Panel>
          <p class="hint">A job takes a copy of these assignments when it starts. Saving affects new jobs only.</p>
          <SaveBar {dirty} {busy} onsave={save} ondiscard={discard} />
        {/if}
      </div>
    </div>
  {/if}
</div>
