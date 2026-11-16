<script lang="ts">
  // A saved agent (#141, R23): Settings (the form, marking what a matching
  // profile overrides or tightens), Jobs (history) and Versions (with diffs).
  // `#/agents/new` is the same form, empty. Skills come later.
  import { untrack } from 'svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import Tabs from '../components/Tabs.svelte';
  import Callout from '../components/Callout.svelte';
  import Dialog from '../components/Dialog.svelte';
  import SaveBar from '../components/SaveBar.svelte';
  import StatusPill from '../components/StatusPill.svelte';
  import ModelInput from '../components/ModelInput.svelte';
  import { app, reloadAgents, agentByName } from '../lib/store.svelte';
  import { route, href, go } from '../lib/router.svelte';
  import { get, post, put, del } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { MODE_HINT, modes } from '../lib/modes';
  import { runtimes, runtimeLabel } from '../lib/runtimes';
  import { canChooseModel, validModel } from '../lib/models';
  import { agoText } from '../lib/format';
  import { STATUS_LABEL, fromForm, profileMarks, toForm, usesAgent, versionDiff, type AgentForm, type ProfileLimits } from '../lib/agents';
  import type { AgentDef } from '../lib/types';

  let { name }: { name: string } = $props();
  const isNew = $derived(name === 'new');
  const agent = $derived(isNew ? undefined : agentByName(name));

  const SECTIONS = ['settings', 'jobs', 'memory', 'versions'];
  const section = $derived(SECTIONS.includes(route.query.get('s') ?? '') ? route.query.get('s')! : 'settings');
  const tabItems = $derived(isNew ? [] : [
    { id: 'settings', label: 'Settings', href: href(`/agents/${name}`) },
    { id: 'jobs', label: 'Jobs', href: href(`/agents/${name}`, { s: 'jobs' }) },
    { id: 'memory', label: 'Memory', href: href(`/agents/${name}`, { s: 'memory' }) },
    { id: 'versions', label: 'Versions', href: href(`/agents/${name}`, { s: 'versions' }) },
  ]);

  // ---- the form -------------------------------------------------------------
  let form = $state<AgentForm>(toForm());
  let base = $state('');
  let loadedId = '';
  // Fill the form once per agent, and again after a save (the version changes).
  $effect(() => {
    const a = agent;
    const key = a ? `${a.id}:${a.version}` : 'new';
    if (key === loadedId) return;
    loadedId = key;
    untrack(() => {
      form = toForm(a);
      if (!a) form.runtime = String(app.state?.config.defaultRuntime ?? 'claude-code');
      base = JSON.stringify(form);
    });
  });
  const dirty = $derived(JSON.stringify(form) !== base);

  const rts = $derived(runtimes(app.state?.plugins));
  const canModel = $derived(canChooseModel(app.state?.plugins, form.runtime));
  const modelOk = $derived(validModel(form.model));
  const nameOk = $derived(/^[a-z0-9][a-z0-9-]{0,40}$/.test(form.name));
  const ready = $derived((!isNew || nameOk) && modelOk && form.runtime !== '' && Number(form.maxLiveJobs) >= 1);

  // ---- what a matching profile does to this agent (R13, R23) ------------------
  // An agent names no repository now, so there is no profile to read here: the
  // marks appear on the job's own page, where the repository is known.
  const profile: ProfileLimits | null = null;
  const marks = $derived(profileMarks({
    autonomy: (form.autonomy || undefined) as AgentDef['autonomy'],
    permissionMode: (form.permissionMode || undefined) as AgentDef['permissionMode'],
    sensitivePaths: form.sensitivePaths.split('\n').filter((s) => s.trim()),
    maxDiffLines: Number(form.maxDiffLines) || undefined,
    jobTokenBudget: Number(form.jobTokenBudget) || undefined,
  }, profile));

  // ---- actions --------------------------------------------------------------
  let busy = $state(false);
  async function save() {
    busy = true;
    try {
      if (isNew) {
        const a = await post<AgentDef>('/agents', fromForm(form));
        await reloadAgents();
        go(`/agents/${a.name}`);
      } else if (agent) {
        await put<AgentDef>(`/agents/${agent.id}`, fromForm(form, agent));
        await reloadAgents();
      }
    } catch (e) {
      showError('The agent was not saved', e);
    } finally {
      busy = false;
    }
  }
  const discard = () => { form = JSON.parse(base); };

  async function archive(on: boolean) {
    if (!agent) return;
    try {
      await post(`/agents/${agent.id}/${on ? 'archive' : 'unarchive'}`);
      await reloadAgents();
    } catch (e) {
      showError(on ? 'Could not archive the agent' : 'Could not bring the agent back', e);
    }
  }
  let confirmDelete = $state(false);
  async function remove() {
    confirmDelete = false;
    if (!agent) return;
    try {
      await del(`/agents/${agent.id}`);
      await reloadAgents();
      go('/agents');
    } catch (e) {
      showError('Could not delete the agent', e);
    }
  }

  // ---- memory (R28) -----------------------------------------------------------
  const MEMORY_MAX = 4000;
  let memory = $state('');
  let memoryBase = $state('');
  let memoryBusy = $state(false);
  $effect(() => {
    const a = agent;
    if (!a || section !== 'memory') return;
    get<{ text: string }>(`/agents/${a.id}/memory`)
      .then((r) => { memory = memoryBase = r.text; })
      .catch((e) => showError('Could not load the memory', e));
  });
  const memoryDirty = $derived(memory !== memoryBase);
  async function saveMemory() {
    if (!agent) return;
    memoryBusy = true;
    try {
      const r = await put<{ text: string }>(`/agents/${agent.id}/memory`, { text: memory });
      memory = memoryBase = r.text;
    } catch (e) {
      showError('Could not save the memory', e);
    } finally {
      memoryBusy = false;
    }
  }

  // ---- jobs and versions ----------------------------------------------------
  const jobs = $derived(agent ? (app.state?.jobs ?? []).filter((j) => usesAgent(j, agent.id)).sort((a, b) => b.createdAt - a.createdAt) : []);

  let versions = $state<AgentDef[] | null>(null);
  $effect(() => {
    const a = agent;
    const sec = section;
    // Refetched after a save too: the version number is part of the key.
    const key = a ? `${a.id}:${a.version}` : '';
    versions = null;
    if (!a || sec !== 'versions') return;
    get<{ versions: AgentDef[] }>(`/agents/${a.id}/versions`)
      .then((r) => { if (key === `${a.id}:${a.version}`) versions = r.versions; })
      .catch((e) => showError('Could not load the versions', e));
  });
  const since = $derived(Number(route.query.get('since')) || 0);
  const sinceDiff = $derived.by(() => {
    const from = versions?.find((v) => v.version === since);
    return from && agent ? versionDiff(from, agent) : null;
  });
  const diffOf = (i: number) => (versions && i + 1 < versions.length ? versionDiff(versions[i + 1], versions[i]) : null);

  const showKnown = (v: string) => (v === '' ? '(not set)' : v);
</script>

<PageHead width="mid" title={isNew ? 'New agent' : (agent?.name ?? name)} sub={agent?.description || undefined}>
  {#snippet meta()}
    {#if agent}<span class="muted num">v{agent.version}</span>
      <span class="pill" class:gate={agent.status === 'needs-you'} class:live={agent.status === 'working'}>{STATUS_LABEL[agent.status]}</span>{/if}
  {/snippet}
  {#snippet actions()}
    {#if agent}
      {#if agent.status !== 'archived'}
        <a class="btn primary" href={href('/new', { agent: agent.name })} aria-disabled={!!agent.broken}>Start job</a>
      {/if}
    {/if}
  {/snippet}
  {#snippet tabs()}
    {#if !isNew}<Tabs items={tabItems} current={section} label="Agent sections" />{/if}
  {/snippet}
</PageHead>

{#if !isNew && !agent}
  <div class="stack mid"><p class="empty">No agent {name}. <a href={href('/agents')}>Back to the agents</a>.</p></div>
{:else}
<div class="stack mid">
  {#if agent?.broken}
    <Callout tone="bad">Jobs cannot start from this agent: {agent.broken}. Fix its settings or switch the plugin back on.</Callout>
  {/if}
  {#if agent?.status === 'archived'}
    <Callout tone="plain">Archived: hidden from pickers. Its jobs keep working.
      <button class="btn" onclick={() => archive(false)}>Bring it back</button></Callout>
  {/if}

  {#if section === 'settings'}
    <Panel title="Settings" hint="Saving makes a new version. Jobs already started keep the version they started with.">
      <div class="g2">
        <label class="f">Name
          <input class="mono" bind:value={form.name} disabled={!isNew} placeholder="reviewer" autocomplete="off"
            oninput={() => (form.name = form.name.toLowerCase())} />
          {#if isNew && form.name && !nameOk}<span class="hint err">Lowercase letters, digits and -, up to 41.</span>
          {:else}<span class="hint">{isNew ? 'Jobs are named after it: <name>-<n>. It cannot change later.' : 'The name cannot change.'}</span>{/if}</label>
        <label class="f">Description
          <input bind:value={form.description} maxlength="200" placeholder="Reviews payment changes" /></label>
      </div>
      <div class="g2">
        <label class="f">Runtime
          <select bind:value={form.runtime}>
            {#each rts as r (r.name)}<option value={r.name}>{runtimeLabel(r)}</option>{/each}
            {#if form.runtime && !rts.some((r) => r.name === form.runtime)}<option value={form.runtime}>{form.runtime} (off)</option>{/if}
          </select></label>
        {#if canModel}
          <label class="f">Model
            <ModelInput runtime={form.runtime} bind:value={form.model} />
            {#if !modelOk}<span class="hint err">Letters, digits and . _ : / @ - [ ] only.</span>{/if}</label>
        {/if}
      </div>
      <label class="f">Fallback runtime
        <select bind:value={form.fallbackRuntime}>
          <option value="">none</option>
          {#each rts.filter((r) => r.name !== form.runtime) as r (r.name)}<option value={r.name}>{runtimeLabel(r)}</option>{/each}
        </select>
        <span class="hint">A job starts on it when the runtime above is switched off at that moment.</span></label>
      <div class="g2">
        <label class="f">Autonomy
          <select bind:value={form.autonomy}><option value="">global setting</option><option value="ask">ask</option><option value="important">important</option></select>
          {@render mark(marks.autonomy)}</label>
        <label class="f">Tool permissions
          <select bind:value={form.permissionMode}>
            <option value="">global setting</option>
            {#each modes(!!app.state?.sandbox) as m (m)}<option value={m}>{m}</option>{/each}
          </select>
          <span class="hint">{form.permissionMode ? MODE_HINT[form.permissionMode as keyof typeof MODE_HINT] : 'The mode set in Settings → Gates & autonomy.'}</span>
          {@render mark(marks.permissionMode)}</label>
      </div>
      <label class="f">Instructions
        <textarea rows="6" bind:value={form.instructions} placeholder="Added to every job's instructions, after the repository's"></textarea>
        <span class="hint">Jobs read this text as their own instructions: write only what you trust.</span></label>
      <div class="g2">
        <label class="f">Live jobs at once
          <input type="number" min="1" max="100" bind:value={form.maxLiveJobs} />
          <span class="hint">More wait in the queue until one finishes or pauses.</span></label>
        <label class="f">Token budget per job
          <input type="number" min="0" bind:value={form.jobTokenBudget} placeholder="off" />
          {@render mark(marks.jobTokenBudget)}</label>
      </div>
      <div class="g2">
        <label class="f">Max diff lines per commit
          <input type="number" min="0" bind:value={form.maxDiffLines} placeholder="the repository's" />
          {@render mark(marks.maxDiffLines)}</label>
        <label class="f">Sensitive paths (one per line)
          <textarea rows="3" class="mono" bind:value={form.sensitivePaths} placeholder="migrations/**"></textarea>
          <span class="hint">Added to the repository's. A commit touching one waits for you.</span>
          {@render mark(marks.sensitivePaths)}</label>
      </div>
      {#snippet foot()}
        {#if isNew}
          <a class="btn ghost" href={href('/agents')}>Cancel</a>
          <button class="btn primary" disabled={!ready || busy} onclick={save}>{busy ? 'Creating…' : 'Create agent'}</button>
        {/if}
      {/snippet}
    </Panel>
    {#if !isNew}
      <SaveBar dirty={dirty} {busy} onsave={save} ondiscard={discard} />
      <Panel title="Archive or delete">
        <p class="hint">Archiving hides the agent from pickers and keeps its history. Deleting needs no live job; its jobs keep their settings and show its name as plain text.</p>
        <div class="row">
          {#if agent?.status === 'archived'}
            <button class="btn" onclick={() => archive(false)}>Unarchive</button>
          {:else}
            <button class="btn" onclick={() => archive(true)}>Archive</button>
          {/if}
          <button class="btn danger" onclick={() => (confirmDelete = true)}>Delete…</button>
        </div>
      </Panel>
    {/if}
  {:else if section === 'jobs'}
    <Panel title="Jobs" hint="Newest first." flush>
      {#if jobs.length}
        <div class="tscroll">
          <table class="tbl">
            <thead><tr><th>Job</th><th>Status</th><th class="hide-sm">Version</th><th class="r">Started</th></tr></thead>
            <tbody>
              {#each jobs as j (j.id)}
                <tr class="hov" onclick={() => go(`/jobs/${j.name}`)}>
                  <td><a class="mono" href={href(`/jobs/${j.name}`)}>{j.name}</a></td>
                  <td><StatusPill agent={j} /></td>
                  <td class="hide-sm num">v{agent ? j.agents?.[agent.id]?.version : ''}{#if agent && j.agents?.[agent.id] && j.agents[agent.id].version < agent.version}<span class="muted"> (now v{agent.version})</span>{/if}</td>
                  <td class="r muted num">{agoText(j.createdAt)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <div class="cb"><p class="empty">No job has started from this agent. <a href={href('/new', { agent: agent?.name })}>Start one</a>.</p></div>
      {/if}
    </Panel>
  {:else if section === 'memory'}
    <Panel title="Memory" hint="What every job from this agent should know. Jobs get the text as it is when they start; changing it later does not reach jobs already started.">
      <label class="f">Memory
        <textarea rows="10" bind:value={memory} maxlength={MEMORY_MAX} placeholder="Where the tests live, how this team names things…"></textarea>
        <span class="hint">{memory.length} / {MEMORY_MAX} characters. It goes into each job's instructions, after the agent's own.</span>
      </label>
      {#snippet foot()}
        <button class="btn ghost" disabled={!memoryDirty || memoryBusy} onclick={() => (memory = memoryBase)}>Discard</button>
        <button class="btn primary" disabled={!memoryDirty || memoryBusy} onclick={saveMemory}>{memoryBusy ? 'Saving…' : 'Save memory'}</button>
      {/snippet}
    </Panel>
  {:else}
    {#if since && sinceDiff}
      <Panel title="Changed since version {since}" hint="What a job started from version {since} did not get.">
        {@render diffTable(sinceDiff)}
      </Panel>
    {/if}
    <Panel title="Versions" hint="Every edit is kept. Newest first.">
      {#if !versions}<p class="muted">Loading…</p>
      {:else}
        <ol class="versions">
          {#each versions as v, i (v.version)}
            <li>
              <div class="row"><b class="num">v{v.version}</b>{#if i === 0}<span class="pill">current</span>{/if}<span class="muted">{agoText(v.updatedAt)}</span></div>
              {#if i === versions.length - 1}<p class="hint">Created.</p>
              {:else}{@render diffTable(diffOf(i) ?? [])}{/if}
            </li>
          {/each}
        </ol>
      {/if}
    </Panel>
  {/if}
</div>
{/if}

{#if confirmDelete && agent}
  <Dialog title="Delete {agent.name}?" onclose={() => (confirmDelete = false)}>
    <p>The agent, its versions and its memory are deleted. Its jobs keep running and keep their settings; they show the name as plain text. This cannot be undone: archive it to keep the history.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (confirmDelete = false)}>Keep it</button>
      <button class="btn danger" onclick={remove}>Delete agent</button>
    {/snippet}
  </Dialog>
{/if}

{#snippet mark(m: import('../lib/agents').ProfileMark | undefined)}
  {#if m}<span class="hint pm" class:low={m.lowered}>Profile <b>{m.profile}</b>: {m.text}.</span>{/if}
{/snippet}

{#snippet diffTable(rows: import('../lib/agents').FieldDiff[])}
  {#if rows.length}
    <table class="tbl dt">
      <thead><tr><th>Setting</th><th>Before</th><th>After</th></tr></thead>
      <tbody>
        {#each rows as r (r.field)}
          <tr><td>{r.label}</td><td class="wrap-any del">{showKnown(r.from)}</td><td class="wrap-any ins">{showKnown(r.to)}</td></tr>
        {/each}
      </tbody>
    </table>
  {:else}<p class="hint">No setting changed.</p>{/if}
{/snippet}

<style>
  .versions { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 16px; }
  .pm.low { color: var(--ink); font-weight: 600; }
  .dt .del { text-decoration: line-through; text-decoration-color: var(--line); }
</style>
