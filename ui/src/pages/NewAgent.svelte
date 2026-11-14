<script lang="ts">
  // New agent (POST /agents): a name, the workflow, an optional ticket, the
  // repository and instructions; the rest under "More options" (#114, #115).
  import { onMount } from 'svelte';
  import { get, post } from '../lib/api';
  import { upsertAgent, app } from '../lib/store.svelte';
  import { go, href } from '../lib/router.svelte';
  import { showError } from '../lib/errors.svelte';
  import type { Agent } from '../lib/types';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import Seg from '../components/Seg.svelte';
  import { MODE_HINT, modes, type PermissionMode } from '../lib/modes';
  import { runtimes, runtimeLabel } from '../lib/runtimes';
  import ModelInput from '../components/ModelInput.svelte';
  import { canChooseModel, validModel } from '../lib/models';

  const flows = $derived([...new Set((app.state?.flows ?? []).map((f) => f.flow))]);
  let f = $state({ name: '', ticket: '', repo: '', base: '', flow: 'feature', prompt: '', autonomy: 'ask', mode: '' as PermissionMode | '', runtime: '', model: '' });
  let busy = $state(false);
  let autonomyTouched = $state(false); // a profile's autonomy fills the selector until the user picks one
  const nameOk = $derived(/^[a-z0-9][a-z0-9-]{0,40}$/.test(f.name));

  // Ticket: a key or an issue URL; the daemon says which tracker reads it
  // and the key a URL stands for (POST /tickets/resolve).
  type TicketFound = { key: string | null; url: string | null; tracker: string | null };
  let tk = $state<{ input: string; data: TicketFound | null; error: string | null }>({ input: '', data: null, error: null });
  const ticketIn = $derived(f.ticket.trim());
  const ticketBad = $derived(tk.input === ticketIn && tk.error !== null);
  async function resolveTicket() {
    const want = ticketIn;
    if (!want) { tk = { input: '', data: null, error: null }; return; }
    tk = { input: want, data: null, error: null };
    try {
      const d = await post<TicketFound>('/tickets/resolve', repo.data ? { ticket: want, repo: repo.data.path } : { ticket: want });
      if (tk.input === want) tk.data = d;
    } catch (e) {
      if (tk.input === want) tk.error = e instanceof Error ? e.message : String(e);
    }
  }
  // Same rule as the daemon's `new::branch_name`: the ticket key (without
  // `#`) prefixes the name, unless the name already starts with it.
  function branchName(key: string | null | undefined, name: string): string {
    const k = key?.replace(/^#/, '');
    if (!k) return name;
    const lower = k.toLowerCase();
    if (name === lower || name.startsWith(`${lower}-`)) return k + name.slice(lower.length);
    return `${k}-${name}`;
  }
  const branch = $derived(branchName(tk.input === ticketIn ? tk.data?.key : null, f.name));

  // More options: open by itself once one of them is changed; the
  // open/closed choice is remembered per browser.
  const MORE = 'xherd.newAgent.more';
  let more = $state((() => { try { return localStorage.getItem(MORE) === '1'; } catch { return false; } })());
  function toggleMore(e: Event) {
    more = (e.currentTarget as HTMLDetailsElement).open;
    try { localStorage.setItem(MORE, more ? '1' : '0'); } catch { /* private mode */ }
  }
  const moreTouched = $derived(autonomyTouched || !!f.runtime || !!f.mode || !!f.model.trim());

  // Repository: a local path or a remote URL, which the daemon clones into
  // the workspace once and fetches later (POST /repos/resolve).
  type Resolved = { path: string; cloned: boolean; warning: string | null; branches: string[]; base: string | null; profile: { id: string; name: string; autonomy?: string; permissionMode?: PermissionMode; runtime?: string; models?: Record<string, string> } | null; suggestedMatch: string };
  let known = $state<{ path: string }[]>([]);
  let repo = $state<{ input: string; busy: boolean; data: Resolved | null; error: string | null }>(
    { input: '', busy: false, data: null, error: null });
  const isHttp = (s: string) => /^https?:\/\//i.test(s);
  const isUrl = (s: string) => s.includes('://') || /^[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:/.test(s);
  const input = $derived(f.repo.trim());
  const repoBad = $derived(repo.input === input && repo.error !== null);
  onMount(async () => {
    try { known = (await get<{ repos: { path: string }[] }>('/repos')).repos; } catch { /* the field still works */ }
  });

  let pending: Promise<Resolved | null> | null = null;
  // One call per input: Create waits for a clone already under way.
  function resolve(): Promise<Resolved | null> {
    const want = input;
    if (!want) return Promise.resolve(null);
    if (repo.input === want && repo.data) return Promise.resolve(repo.data);
    if (repo.input === want && repo.busy && pending) return pending;
    repo = { input: want, busy: true, data: null, error: null };
    pending = fetchRepo(want);
    return pending;
  }
  async function fetchRepo(want: string): Promise<Resolved | null> {
    try {
      const d = await post<Resolved>('/repos/resolve', { repo: want });
      if (repo.input === want) {
        repo.data = d;
        if (d.profile?.autonomy && !autonomyTouched) f.autonomy = d.profile.autonomy;
        // A key's tracker can depend on the repository's host.
        if (ticketIn && !isHttp(ticketIn)) resolveTicket();
      }
      return d;
    } catch (e) {
      if (repo.input === want) repo.error = e instanceof Error ? e.message : String(e);
      return null;
    } finally {
      if (repo.input === want) repo.busy = false;
    }
  }

  // The runtime the agent will get, for its model suggestions and the
  // profile's default model (Settings' own default is applied by the daemon).
  const runtime = $derived(f.runtime || repo.data?.profile?.runtime || String(app.state?.config.defaultRuntime ?? 'claude-code'));
  const canModel = $derived(canChooseModel(app.state?.plugins, runtime));
  const profileModel = $derived(repo.data?.profile?.models?.[runtime]);
  const modelOk = $derived(validModel(f.model));

  // What applies when More options stays closed, and where it comes from.
  const cfg = $derived(app.state?.config ?? {});
  const prof = $derived(repo.data?.profile);
  const summary = $derived.by(() => {
    const rt = runtimes(app.state?.plugins).find((r) => r.name === runtime);
    const model = f.model.trim() || profileModel || (cfg.runtimeModels as Record<string, string> | undefined)?.[runtime] || '';
    const mode = f.mode || prof?.permissionMode || String(cfg.permissionMode ?? 'edits');
    return [f.autonomy, rt ? runtimeLabel(rt) : runtime, ...(canModel && model ? [model] : []), mode].join(' · ');
  });

  const ready = $derived(nameOk && !ticketBad && modelOk && input !== '' && !repoBad && (f.flow !== 'none' || f.prompt.trim() !== ''));

  async function create(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const r = await resolve();
    if (!r) { busy = false; return; }
    const body: Record<string, string> = { name: f.name, repo: r.path, flow: f.flow };
    if (ticketIn) body.ticket = ticketIn;
    if (f.base.trim()) body.base = f.base.trim();
    if (f.prompt.trim()) body.prompt = f.prompt.trim();
    body.autonomy = f.autonomy;
    if (f.mode) body.permissionMode = f.mode;
    if (f.runtime) body.runtime = f.runtime;
    if (canModel && f.model.trim()) body.model = f.model.trim();
    try {
      const r = await post<{ agent: Agent }>('/agents', body);
      upsertAgent(r.agent);
      go(`/agent/${r.agent.name}`);
    } catch (err) {
      showError('Could not create the agent', err);
    } finally {
      busy = false;
    }
  }
</script>

<PageHead width="narrow" title="New agent" sub="One agent, one worktree, one branch." />
<form class="stack narrow" onsubmit={create}>
  <Panel>
    <div class="g2">
      <label class="f">Name
        <input class="mono" bind:value={f.name} placeholder="refund-retry" required autocomplete="off"
          oninput={() => (f.name = f.name.toLowerCase())} />
        {#if f.name && !nameOk}<span class="hint err">Lowercase letters, digits and -, up to 41.</span>
        {:else if f.name}<span class="hint">Branch <span class="mono">{branch}</span></span>
        {:else}<span class="hint">Agent, session and worktree folder.</span>{/if}</label>
      <div class="f">Workflow
        <Seg label="Workflow" value={f.flow} onchange={(v) => (f.flow = v)}
          options={[...flows, 'none'].map((fl) => ({ value: fl, label: fl === 'none' ? 'No workflow' : fl }))} /></div>
    </div>
    <label class="f">Ticket (optional)
      <input class="mono" bind:value={f.ticket} onchange={resolveTicket} autocomplete="off"
        placeholder="https://acme.atlassian.net/browse/PAY-412 or PAY-412" />
      {#if ticketBad}<span class="hint err">✕ {tk.error}</span>
      {:else if tk.data && tk.input === ticketIn}
        {#if tk.data.tracker}<span class="hint">✓ <span class="mono">{tk.data.key ?? ticketIn}</span> · read by {tk.data.tracker}</span>
        {:else if tk.data.url}<span class="hint">No issue tracker reads this URL: it is kept as a link.</span>
        {:else}<span class="hint"><span class="mono">{tk.data.key}</span> · no issue tracker reads this key: it names the branch only.</span>{/if}
      {:else}<span class="hint">An issue URL or a key. Its key starts the branch name.</span>{/if}</label>
    <div class="g2">
      <label class="f">Repository: URL or local path
        <input class="mono" bind:value={f.repo} list="known-repos" required onchange={resolve}
          placeholder="https://github.com/acme/payments-api or /Users/you/xherd/payments-api" />
        <datalist id="known-repos">{#each known as k (k.path)}<option value={k.path}></option>{/each}</datalist>
        {#if repo.busy && repo.input === input}
          <span class="hint">{isUrl(input) ? `Cloning or fetching ${input}…` : 'Checking…'}</span>
        {:else if repoBad}
          <span class="hint err">✕ {repo.error}</span>
        {:else if repo.data && repo.input === input}
          <span class="hint wrap-any">✓ <span class="mono">{repo.data.path}</span>{repo.data.cloned ? ' (cloned now)' : ''}</span>
          {#if repo.data.warning}<span class="hint wrap-any">⚠ {repo.data.warning}</span>{/if}
        {:else}
          <span class="hint">A URL is cloned into the workspace once and fetched before each new agent.</span>
        {/if}</label>
      <label class="f">Base branch (optional)
        <input class="mono" bind:value={f.base} list="repo-branches"
          placeholder={repo.data?.base ?? "origin's default branch"} />
        <datalist id="repo-branches">{#each repo.data?.branches ?? [] as b (b)}<option value={b}></option>{/each}</datalist></label>
    </div>
    {#if repo.data?.profile}<span class="hint">Profile <b>{repo.data.profile.name}</b> applies.</span>
    {:else if repo.data}<span class="hint">No profile for this repository, so the global settings apply.
      <a href={href('/settings', { tab: 'profiles', new: repo.data.suggestedMatch })}>Create one</a> for a base branch, test command or sensitive paths.</span>{/if}
    <label class="f">{f.flow === 'none' ? 'Instructions' : 'Instructions (optional)'}
      <textarea rows="4" bind:value={f.prompt} required={f.flow === 'none'}
        placeholder={f.flow === 'none' ? 'Typed into the agent as its first message' : 'Context for the spec step'}></textarea></label>
    <details class="more" open={more || moreTouched} ontoggle={toggleMore}>
      <summary><span class="mono hint">{summary}{prof ? ` (profile ${prof.name})` : ''}</span><span class="lbl">More options</span></summary>
      <div class="opts">
      <div class="f">Autonomy
        <Seg label="Autonomy" value={f.autonomy} onchange={(v) => { autonomyTouched = true; f.autonomy = v; }}
          options={[{ value: 'ask', label: 'ask' }, { value: 'important', label: 'important' }]} />
        <span class="hint">{f.autonomy === 'ask'
          ? 'Every gate waits for you.'
          : 'Design, tasks, review and commit gates are approved by xherd when nothing needs you; spec and ship always wait.'}</span></div>
      <label class="f">Runtime
        <select bind:value={f.runtime}>
          <option value="">{repo.data?.profile?.runtime ? `${repo.data.profile.runtime} (profile)` : 'Default (Settings)'}</option>
          {#each runtimes(app.state?.plugins) as r (r.name)}<option value={r.name}>{runtimeLabel(r)}</option>{/each}
        </select>
        <span class="hint">The coding agent that runs in the terminal. Set the default in Settings → Gates &amp; autonomy.</span></label>
      {#if canModel}
        <label class="f">Model (optional)
          <ModelInput {runtime} bind:value={f.model} placeholder={profileModel ? `${profileModel} (profile)` : 'Default (Settings, else the runtime’s)'} />
          {#if !modelOk}<span class="hint err">Letters, digits and . _ : / @ - [ ] only.</span>
          {:else}<span class="hint">Pick one, or Other… for any model {runtime} knows. Set a default per runtime in Settings → Plugins → Options. Each agent can change it later.</span>{/if}</label>
      {/if}
      <label class="f">Tool permissions
        <select bind:value={f.mode}>
          <option value="">{repo.data?.profile?.permissionMode ? `${repo.data.profile.permissionMode} (profile)` : 'Default (Settings)'}</option>
          {#each modes(!!app.state?.sandbox) as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <span class="hint">{f.mode || repo.data?.profile?.permissionMode ? MODE_HINT[(f.mode || repo.data?.profile?.permissionMode)!] : 'The mode set in Settings → Gates & autonomy (edits unless you changed it).'} Each agent can change it later.</span></label>
      </div>
    </details>
    {#snippet foot()}
      <button type="button" class="btn ghost" onclick={() => history.back()}>Cancel</button>
      <button class="btn primary" disabled={!ready || busy}>{busy && repo.busy ? 'Getting the repository…' : 'Create agent'}</button>
    {/snippet}
  </Panel>
</form>

<style>
  .more > summary { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; flex-wrap: wrap; cursor: pointer; list-style: none; }
  .more > summary::-webkit-details-marker { display: none; }
  .more > summary .lbl::after { content: ' ▸'; }
  .more[open] > summary .lbl::after { content: ' ▾'; }
  .opts { display: flex; flex-direction: column; gap: 14px; margin-top: 14px; }
</style>
