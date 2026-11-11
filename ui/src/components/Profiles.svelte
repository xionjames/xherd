<script lang="ts">
  // Repo profiles (spec §16.1): per-repository settings, matched by remote URL
  // then path when an agent is created. An empty field uses the global setting.
  import { untrack } from 'svelte';
  import { get, post, put, del } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { route } from '../lib/router.svelte';

  type Profile = {
    id: string; name: string; match: string[];
    baseBranch?: string; stagingBranch?: string; hotfixBase?: string;
    autonomy?: string; sensitivePaths?: string[]; maxDiffLines?: number;
    agentTokenBudget?: number; testCommand?: string; lintCommand?: string; instructions?: string;
  };
  type Form = {
    id: string; name: string; match: string; baseBranch: string; stagingBranch: string; hotfixBase: string;
    autonomy: string; sensitivePaths: string; maxDiffLines: string; agentTokenBudget: string;
    testCommand: string; lintCommand: string; instructions: string;
  };

  let profiles = $state<Profile[]>([]);
  let loaded = $state(false);
  let form = $state<Form | null>(null);
  let busy = $state(false);
  let importText = $state('');
  let importing = $state(false);

  const blank = (): Form => ({ id: '', name: '', match: '', baseBranch: '', stagingBranch: '', hotfixBase: '', autonomy: '', sensitivePaths: '', maxDiffLines: '', agentTokenBudget: '', testCommand: '', lintCommand: '', instructions: '' });
  const lines = (s: string) => s.split('\n').map((x) => x.trim()).filter(Boolean);
  const opt = (s: string) => (s.trim() ? s.trim() : undefined);
  const num = (s: string) => (s.trim() === '' ? undefined : Number(s));

  function toForm(p: Profile): Form {
    return {
      id: p.id, name: p.name, match: p.match.join('\n'), baseBranch: p.baseBranch ?? '', stagingBranch: p.stagingBranch ?? '',
      hotfixBase: p.hotfixBase ?? '', autonomy: p.autonomy ?? '',
      sensitivePaths: p.sensitivePaths ? p.sensitivePaths.join('\n') : '', maxDiffLines: p.maxDiffLines?.toString() ?? '',
      agentTokenBudget: p.agentTokenBudget?.toString() ?? '', testCommand: p.testCommand ?? '', lintCommand: p.lintCommand ?? '', instructions: p.instructions ?? '',
    };
  }
  // An empty "sensitive paths" box means the global list; the checkbox below
  // makes it an explicit empty list (the profile switches the tripwire off).
  let noSensitive = $state(false);
  function edit(p: Profile | null) {
    form = p ? toForm(p) : blank();
    noSensitive = !!p && Array.isArray(p.sensitivePaths) && p.sensitivePaths.length === 0;
  }
  function body(f: Form) {
    return {
      name: f.name, match: lines(f.match),
      baseBranch: opt(f.baseBranch), stagingBranch: opt(f.stagingBranch), hotfixBase: opt(f.hotfixBase),
      autonomy: opt(f.autonomy),
      sensitivePaths: noSensitive ? [] : lines(f.sensitivePaths).length ? lines(f.sensitivePaths) : undefined,
      maxDiffLines: num(f.maxDiffLines), agentTokenBudget: num(f.agentTokenBudget),
      testCommand: opt(f.testCommand), lintCommand: opt(f.lintCommand), instructions: opt(f.instructions),
    };
  }

  async function load() {
    try {
      profiles = (await get<{ profiles: Profile[] }>('/profiles')).profiles;
      loaded = true;
    } catch (e) {
      showError('Could not list profiles', e);
    }
  }
  async function save() {
    if (!form) return;
    busy = true;
    try {
      if (form.id) await put(`/profiles/${form.id}`, body(form));
      else await post('/profiles', body(form));
      form = null;
      await load();
    } catch (e) {
      showError('The profile was not saved', e);
    } finally {
      busy = false;
    }
  }
  async function remove(p: Profile) {
    if (!confirm(`Delete profile ${p.name}? Its agents keep running with the global settings.`)) return;
    try {
      await del(`/profiles/${p.id}`);
      if (form?.id === p.id) form = null;
      await load();
    } catch (e) {
      showError('The profile was not deleted', e);
    }
  }
  async function exportJson() {
    try {
      const v = await get<unknown>('/profiles/export');
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([JSON.stringify(v, null, 2)], { type: 'application/json' }));
      a.download = 'xherd-profiles.json';
      a.click();
      URL.revokeObjectURL(a.href);
    } catch (e) {
      showError('Could not export profiles', e);
    }
  }
  async function doImport() {
    try {
      const r = await post<{ added: number; replaced: number }>('/profiles/import', JSON.parse(importText));
      importText = '';
      importing = false;
      alert(`Imported: ${r.added} added, ${r.replaced} replaced.`);
      await load();
    } catch (e) {
      showError('The import failed', e);
    }
  }
  // Arriving from New agent with ?new=<match>: open the form with it filled in.
  $effect(() => {
    const m = route.query.get('new');
    if (!m) return;
    untrack(() => {
      edit(null);
      if (form) { form.match = m; form.name = m.split('/').filter(Boolean).pop() ?? ''; }
    });
  });
  $effect(() => { load(); });
</script>

<div class="secs">
  <section class="sec">
    <div class="sh"><h2>Repo profiles</h2><span class="hint">Settings per repository, found by its remote URL, then its path, when an agent is created. An empty field uses the global setting.</span></div>
    <div class="bd">
      {#if loaded && !profiles.length}<p class="muted">No profiles yet. Every repository uses the global settings.</p>{/if}
      {#each profiles as p (p.id)}
        <div class="card" style="display:flex;gap:12px;align-items:center">
          <div style="flex:1;min-width:0">
            <b>{p.name}</b>
            <div class="hint mono" style="overflow-wrap:anywhere">{p.match.join(' · ')}</div>
          </div>
          <button class="btn sm" onclick={() => edit(p)}>Edit</button>
          <button class="btn ghost sm" onclick={() => remove(p)}>Delete</button>
        </div>
      {/each}
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <button class="btn primary" onclick={() => edit(null)}>New profile</button>
        <button class="btn" onclick={exportJson} disabled={!profiles.length}>Export JSON</button>
        <button class="btn" onclick={() => (importing = !importing)}>Import JSON</button>
      </div>
      {#if importing}
        <label class="f">Exported profiles
          <textarea class="mono" rows="6" bind:value={importText} placeholder={'{"profiles": [...]}'}></textarea>
          <span class="hint">A profile with an existing name replaces it. Nothing is imported if one is invalid.</span></label>
        <div><button class="btn" disabled={!importText.trim()} onclick={doImport}>Import</button></div>
      {/if}
    </div>
  </section>

  {#if form}
    <section class="sec">
      <div class="sh"><h2>{form.id ? 'Edit profile' : 'New profile'}</h2></div>
      <div class="bd" style="gap:0">
        <div class="grp" style="display:flex;flex-direction:column;gap:16px">
          <div><h3 style="margin:0;font-size:14px">Profile</h3><div class="hint">Which repositories it applies to.</div></div>
          <label class="f">Name <input bind:value={form.name} maxlength="60" /></label>
          <label class="f">Applies to
            <textarea class="mono" rows="3" bind:value={form.match} placeholder={'github.com/acme/*\n/work/acme'}></textarea>
            <span class="hint">One per line: a remote URL (<span class="mono">github.com/owner/repo</span>, any scheme, <span class="mono">*</span> allowed) or an absolute folder. A URL match wins over a folder; the longest pattern wins.</span></label>
        </div>
        <div class="grp" style="display:flex;flex-direction:column;gap:16px;border-top:1px solid var(--fence-soft);margin-top:20px;padding-top:20px">
          <div><h3 style="margin:0;font-size:14px">Branches</h3></div>
          <label class="f">Base branch <input class="mono" bind:value={form.baseBranch} placeholder="the repository's default" />
            <span class="hint">Where new agents start when the request names none.</span></label>
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px">
            <label class="f">Staging branch <input class="mono" bind:value={form.stagingBranch} /></label>
            <label class="f">Hotfix base <input class="mono" bind:value={form.hotfixBase} /></label>
          </div>
          <span class="hint">Staging and hotfix branches are stored for the flows that will use them; nothing reads them yet.</span>
        </div>
        <div class="grp" style="display:flex;flex-direction:column;gap:16px;border-top:1px solid var(--fence-soft);margin-top:20px;padding-top:20px">
          <div><h3 style="margin:0;font-size:14px">Autonomy &amp; limits</h3><div class="hint">Empty fields use the global setting.</div></div>
          <label class="f">Autonomy of new agents
            <select bind:value={form.autonomy}><option value="">global setting</option><option value="ask">ask</option><option value="important">important</option></select></label>
          <label class="f">Largest commit
            <span class="unit"><input type="number" min="1" bind:value={form.maxDiffLines} placeholder="global" /><span>changed lines</span></span></label>
          <label class="f">Token budget per agent
            <span class="unit"><input type="number" min="0" bind:value={form.agentTokenBudget} placeholder="global" /><span>tokens</span></span>
            <span class="hint">0 turns it off for this repository.</span></label>
          <label class="f">Sensitive paths
            <textarea class="mono" rows="3" bind:value={form.sensitivePaths} disabled={noSensitive} placeholder="global list"></textarea>
            <span class="hint">Globs, one per line; a commit touching one waits for you. Replaces the global list.</span></label>
          <label class="f chk"><input type="checkbox" bind:checked={noSensitive} /> No sensitive paths for this repository</label>
        </div>
        <div class="grp" style="display:flex;flex-direction:column;gap:16px;border-top:1px solid var(--fence-soft);margin-top:20px;padding-top:20px">
          <div><h3 style="margin:0;font-size:14px">Checks before every commit</h3><div class="hint">Optional. Leave both empty to run nothing.</div></div>
          <p class="hint" style="margin:0">xherd runs these itself, in the worktree, each time an agent proposes a commit: lint first, then test. If one fails or times out, the commit waits for you. The agent cannot skip them.</p>
          <label class="f">Lint command
            <input class="mono" bind:value={form.lintCommand} maxlength="500" placeholder="cargo clippy -- -D warnings" /></label>
          <label class="f">Test command
            <input class="mono" bind:value={form.testCommand} maxlength="500" placeholder="cargo test" />
            <span class="hint">One line each; chain steps with <span class="mono">&amp;&amp;</span>.</span></label>
        </div>
        <div class="grp" style="display:flex;flex-direction:column;gap:16px;border-top:1px solid var(--fence-soft);margin-top:20px;padding-top:20px">
          <div><h3 style="margin:0;font-size:14px">Instructions for the agent</h3></div>
          <label class="f">Instructions
            <textarea rows="6" bind:value={form.instructions} placeholder={'How to build: make build\nHow to lint: make lint, before every commit\nConventions, folders to avoid…'}></textarea>
            <span class="hint">Advice for the agent, added to its <span class="mono">instructions.md</span> at its next start. Unlike the checks above, xherd does not run these.</span></label>
        </div>
        <div style="display:flex;gap:8px;border-top:1px solid var(--fence-soft);margin-top:20px;padding-top:16px">
          <button class="btn primary" disabled={busy || !form.name.trim() || !form.match.trim()} onclick={save}>{busy ? 'Saving…' : 'Save profile'}</button>
          <button class="btn ghost" onclick={() => (form = null)}>Cancel</button>
        </div>
      </div>
    </section>
  {/if}
</div>
