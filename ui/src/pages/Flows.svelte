<script lang="ts">
  // Flows (D26, R35, R42): which saved agent runs each stage of a workflow, and
  // whether that stage's gate always waits for you. `#/flows` lists the
  // workflows, `#/flows/<pack>/<flow>` edits one. A stage with no agent of its
  // own runs the previous stage's (or the job's, when it starts from one).
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import SaveBar from '../components/SaveBar.svelte';
  import Callout from '../components/Callout.svelte';
  import { app } from '../lib/store.svelte';
  import { route, href } from '../lib/router.svelte';
  import { get, put } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { stageLabel } from '../lib/flow';
  import { assignmentRows, pickable } from '../lib/agents';
  import type { FlowStageView, FlowView, AssignmentRow } from '../lib/types';

  const m = $derived(route.path.match(/^\/flows\/([^/]+)\/([^/]+)$/));
  const pack = $derived(m?.[1] ? decodeURIComponent(m[1]) : '');
  const flowName = $derived(m?.[2] ? decodeURIComponent(m[2]) : '');
  const list = $derived(app.state?.flows ?? []);
  const agents = $derived((app.state?.agents ?? []));

  type Row = { agent: string; human: boolean };
  let view = $state<FlowView | null>(null);
  let rows = $state<Row[]>([]);
  let orig = $state<Row[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const label = (s: FlowStageView) => stageLabel({ ...s, gate: false });
  const toRows = (v: FlowView): Row[] => v.stages.map((s) => ({ agent: s.assigned ? (s.agent ?? '') : '', human: s.human }));

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

  const dirty = $derived(rows.reduce((n, r, i) => n + (r.agent !== orig[i]?.agent || r.human !== orig[i]?.human ? 1 : 0), 0));
  const choices = $derived(agents.filter(pickable));
  // A stage whose agent is archived or gone is still listed, so it can be changed.
  const names = (cur: string) => (cur && !choices.some((a) => a.name === cur) ? [...choices.map((a) => a.name), cur] : choices.map((a) => a.name));

  async function save() {
    if (!view) return;
    busy = true;
    try {
      const body: { assignments: AssignmentRow[] } = {
        assignments: assignmentRows(rows.map((r, i) => ({ stage: view!.stages[i].id, ...r })), agents),
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

{#if !pack}
  <PageHead title="Flows" sub="Give each stage of a workflow to a saved agent." />
  <div class="stack">
    <Panel title="Workflows" flush>
      {#if list.length}
        <ul class="list">
          {#each list as f (f.pack + '/' + f.flow)}
            <li><a class="mono" href={href(`/flows/${f.pack}/${f.flow}`)}>{f.flow}</a>
              <span class="hint">pack {f.pack} · {f.stages.length} stages: {f.stages.map(stageLabel).join(' → ')}</span></li>
          {/each}
        </ul>
      {:else}<div class="cb"><p class="empty">No workflows.</p></div>{/if}
    </Panel>
  </div>
{:else}
  <PageHead title="Flow {flowName}" sub="Pack {pack}. Stages with no agent run the previous stage's agent." width="mid">
    {#snippet actions()}<a class="btn ghost" href={href('/flows')}>All flows</a>{/snippet}
  </PageHead>
  <div class="stack mid">
    {#if error}
      <Callout tone="bad">{error}</Callout>
    {:else if !view}
      <p class="empty">Loading…</p>
    {:else}
      <Panel title="Stages" hint="A job takes a copy of these assignments when it starts." flush>
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Stage</th><th>Agent</th><th>Gate always waits for me</th></tr></thead>
            <tbody>
              {#each view.stages as s, i (s.id)}
                <tr>
                  <td><b>{label(s)}</b> <span class="hint mono">{s.id}</span></td>
                  <td>
                    <select aria-label="Agent for {label(s)}" bind:value={rows[i].agent}>
                      <option value="">{resolved[i] && !rows[i].agent ? `Same as before (${resolved[i]})` : 'No agent'}</option>
                      {#each names(rows[i].agent) as n (n)}<option value={n}>{n}</option>{/each}
                    </select>
                    {#if s.broken && rows[i].agent === (s.agent ?? '') && s.assigned}<div class="hint err">✕ Cannot start jobs: {s.broken}</div>{/if}
                  </td>
                  <td><label class="chk"><input type="checkbox" bind:checked={rows[i].human} disabled={!rows[i].agent} />
                    <span class="hint">{s.class === 'human-only' ? 'Always, for this stage' : 'Even at higher autonomy'}</span></label></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </Panel>
      <SaveBar {dirty} {busy} onsave={save} ondiscard={discard} />
    {/if}
  </div>
{/if}
