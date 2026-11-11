<script lang="ts">
  // New agent on a ticket (POST /agents).
  import { onMount } from 'svelte';
  import { get, post } from '../lib/api';
  import { upsertAgent, app } from '../lib/store.svelte';
  import { go, href } from '../lib/router.svelte';
  import { showError } from '../lib/errors.svelte';
  import type { Agent } from '../lib/types';

  const flows = $derived([...new Set((app.state?.flows ?? []).map((f) => f.flow))]);
  let f = $state({ ticketKey: '', slug: '', repo: '', base: '', flow: 'feature', prompt: '', autonomy: 'ask' });
  let busy = $state(false);
  let autonomyTouched = false; // a profile's autonomy fills the selector until the user picks one
  const keyOk = $derived(/^[A-Z0-9][A-Z0-9_-]*$/.test(f.ticketKey));
  const slugOk = $derived(!f.slug || /^[a-z0-9][a-z0-9-]{0,40}$/.test(f.slug));
  const name = $derived(f.ticketKey + (f.slug ? `-${f.slug}` : ''));

  // Repository: a local path or a remote URL, which the daemon clones into
  // the workspace once and fetches later (POST /repos/resolve).
  type Resolved = { path: string; cloned: boolean; warning: string | null; branches: string[]; base: string | null; profile: { id: string; name: string; autonomy?: string } | null; suggestedMatch: string };
  let known = $state<{ path: string }[]>([]);
  let repo = $state<{ input: string; busy: boolean; data: Resolved | null; error: string | null }>(
    { input: '', busy: false, data: null, error: null });
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
      }
      return d;
    } catch (e) {
      if (repo.input === want) repo.error = e instanceof Error ? e.message : String(e);
      return null;
    } finally {
      if (repo.input === want) repo.busy = false;
    }
  }

  const ready = $derived(keyOk && slugOk && input !== '' && !repoBad && (f.flow !== 'none' || f.prompt.trim() !== ''));

  async function create(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    const r = await resolve();
    if (!r) { busy = false; return; }
    const body: Record<string, string> = { ticketKey: f.ticketKey, repo: r.path, flow: f.flow };
    if (f.slug) body.slug = f.slug;
    if (f.base.trim()) body.base = f.base.trim();
    if (f.prompt.trim()) body.prompt = f.prompt.trim();
    body.autonomy = f.autonomy;
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

<div class="phead notabs"><h1>New agent</h1><p class="muted">One ticket, one worktree, one agent.</p></div>
<form class="stack" onsubmit={create}>
  <section class="sec"><div class="bd">
    <div class="g2">
      <label class="f">Ticket key
        <input class="mono" bind:value={f.ticketKey} placeholder="PAY-412" required oninput={() => (f.ticketKey = f.ticketKey.toUpperCase())} />
        {#if f.ticketKey && !keyOk}<span class="hint" style="color:var(--halt)">Capital letters, digits, - and _.</span>{/if}</label>
      <label class="f">Short name (optional)
        <input class="mono" bind:value={f.slug} placeholder="refund-retry" />
        {#if !slugOk}<span class="hint" style="color:var(--halt)">Lowercase letters, digits and -, up to 41.</span>
        {:else if f.ticketKey}<span class="hint">Agent, branch and session: <span class="mono">{name}</span></span>{/if}</label>
    </div>
    <label class="f">Repository: URL or local path
      <input class="mono" bind:value={f.repo} list="known-repos" required onchange={resolve}
        placeholder="https://github.com/acme/payments-api or /Users/you/xherd/payments-api" />
      <datalist id="known-repos">{#each known as k (k.path)}<option value={k.path}></option>{/each}</datalist>
      {#if repo.busy && repo.input === input}
        <span class="hint">{isUrl(input) ? `Cloning or fetching ${input}…` : 'Checking…'}</span>
      {:else if repoBad}
        <span class="hint" style="color:var(--halt);word-break:break-word">✕ {repo.error}</span>
      {:else if repo.data && repo.input === input}
        <span class="hint">✓ <span class="mono">{repo.data.path}</span>{repo.data.cloned ? ' (cloned now)' : ''}</span>
        {#if repo.data.warning}<span class="hint" style="word-break:break-word">⚠ {repo.data.warning}</span>{/if}
      {:else}
        <span class="hint">A URL is cloned into the workspace once and fetched before each new agent.</span>
      {/if}</label>
    <label class="f" style="max-width:420px">Base branch (optional)
      <input class="mono" bind:value={f.base} list="repo-branches"
        placeholder={repo.data?.base ?? "origin's default branch"} />
      {#if repo.data?.profile}<span class="hint">Profile <b>{repo.data.profile.name}</b> applies.</span>
      {:else if repo.data}<span class="hint">No profile for this repository, so the global settings apply.
        <a href={href('/settings', { tab: 'profiles', new: repo.data.suggestedMatch })}>Create one</a> for a base branch, test command or sensitive paths.</span>{/if}
      <datalist id="repo-branches">{#each repo.data?.branches ?? [] as b (b)}<option value={b}></option>{/each}</datalist></label>
    <div class="f">Workflow
      <div class="seg" role="group" aria-label="Workflow" style="align-self:flex-start">
        {#each [...flows, 'none'] as fl (fl)}
          <button type="button" class:on={f.flow === fl} aria-pressed={f.flow === fl} onclick={() => (f.flow = fl)}>{fl === 'none' ? 'No workflow' : fl}</button>
        {/each}
      </div></div>
    <div class="f">Autonomy
      <div class="seg" role="group" aria-label="Autonomy" style="align-self:flex-start">
        {#each ['ask', 'important'] as lv (lv)}
          <button type="button" class:on={f.autonomy === lv} aria-pressed={f.autonomy === lv} onclick={() => { autonomyTouched = true; f.autonomy = lv; }}>{lv}</button>
        {/each}
      </div>
      <span class="hint">{f.autonomy === 'ask'
        ? 'Every gate waits for you.'
        : 'Design, tasks, review and commit gates are approved by xherd when nothing needs you; spec and ship always wait.'}</span></div>
    <label class="f">{f.flow === 'none' ? 'Prompt' : 'What to do (optional)'}
      <textarea rows="4" bind:value={f.prompt} placeholder={f.flow === 'none' ? 'Typed into the agent as its first message' : 'Context for the spec step'}></textarea></label>
  </div></section>
  <div class="row"><span class="grow"></span>
    <button type="button" class="btn ghost" onclick={() => history.back()}>Cancel</button>
    <button class="btn primary lg" disabled={!ready || busy}>{busy && repo.busy ? 'Getting the repository…' : 'Create agent'}</button></div>
</form>
