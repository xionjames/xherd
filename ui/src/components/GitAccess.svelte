<script lang="ts">
  // Git access per host (spec R-685, R-686): how HTTPS gets its token (gh or
  // glab login, or a token kept by xherd), SSH, optional URL rewrites, and the
  // doctor with a fix for the first failure. Used by Settings and Welcome.
  import { get, put, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';
  import Panel from './Panel.svelte';
  import SaveBar from './SaveBar.svelte';
  import Callout from './Callout.svelte';
  import type { GitHost, GitHostsView, GitFinding, ConfigView } from '../lib/types';

  type Row = GitHost & { token: string };
  let view = $state<GitHostsView | null>(null);
  let rows = $state<Row[]>([]);
  let saved = $state('');
  let busy = $state(false);
  let newHost = $state('');
  let doctor = $state<Record<string, { repo: string; findings: GitFinding[] | null; running: boolean }>>({});
  let login = $state<string | null>(null);

  const CRED: [GitHost['credential'], string][] = [
    ['gh', 'GitHub CLI login (gh)'],
    ['glab', 'GitLab CLI login (glab)'],
    ['xherd', 'A token kept by xherd'],
    ['none', 'git decides (your own config)'],
  ];
  const PRESETS = ['github.com', 'gitlab.com', 'bitbucket.org'];

  const strip = (r: Row): GitHost => {
    const { token: _t, hasToken: _h, ...h } = r;
    return { ...h, username: h.username || undefined, caFile: h.caFile || undefined };
  };
  const snapshot = () => JSON.stringify(rows.map(strip));
  const dirty = $derived(snapshot() !== saved || rows.some((r) => r.token !== ''));

  function fill(v: GitHostsView) {
    view = v;
    rows = v.hosts.map((h) => ({ ...h, username: h.username ?? '', caFile: h.caFile ?? '', token: '' }));
    saved = snapshot();
  }
  async function load() {
    try { fill(await get<GitHostsView>('/git/hosts')); checkAll(); } catch (e) { showError('Could not load git access', e); }
  }
  $effect(() => { load(); });

  // Check every host as soon as the screen opens and after each save, so the
  // status in the header is current without pressing "Check access".
  function checkAll() { for (const r of rows) runDoctor(r.host); }
  const status = (d?: { findings: GitFinding[] | null; running: boolean }) =>
    d?.running ? 'checking' : !d?.findings ? 'unknown' : d.findings.every((f) => f.ok) ? 'ok' : 'bad';

  // Who agents commit as: git refuses to commit without it, and the container
  // has no identity of its own.
  let idName = $state('');
  let idEmail = $state('');
  let idRules = $state<{ scope: string; name: string; email: string }[]>([]);
  let idSaved = $state('');
  const idSnapshot = () => `${idName}\n${idEmail}\n${JSON.stringify(idRules)}`;
  let idBusy = $state(false);
  const idDirty = $derived(idSnapshot() !== idSaved);
  async function loadIdentity() {
    try {
      const c = (await get<ConfigView>('/config')).config;
      idName = String(c.gitUserName ?? '');
      idEmail = String(c.gitUserEmail ?? '');
      idRules = ((c.gitIdentities ?? []) as typeof idRules).map((r) => ({ ...r }));
      idSaved = idSnapshot();
    } catch (e) { showError('Could not load the commit identity', e); }
  }
  $effect(() => { loadIdentity(); });
  async function saveIdentity() {
    idBusy = true;
    try {
      await put('/config', { gitUserName: idName, gitUserEmail: idEmail, gitIdentities: idRules });
      idRules = idRules.map((r) => ({ ...r, scope: r.scope.trim().replace(/\/+$/, ''), name: r.name.trim(), email: r.email.trim() }));
      idSaved = idSnapshot();
    } catch (e) { showError('Could not save the commit identity', e); }
    finally { idBusy = false; }
  }

  function add(host: string) {
    host = host.trim().toLowerCase();
    if (!host || rows.some((r) => r.host === host)) return;
    const credential: GitHost['credential'] = host === 'github.com' ? 'gh' : host.includes('gitlab') ? 'glab' : 'xherd';
    rows.push({ host, protocols: ['https', 'ssh'], credential, username: '', caFile: '', preferHttps: false, preferSsh: false, token: '' });
    newHost = '';
  }

  async function save() {
    busy = true;
    const tokens = Object.fromEntries(rows.filter((r) => r.token !== '').map((r) => [r.host, r.token]));
    try {
      fill(await put<GitHostsView>('/git/hosts', { hosts: rows.map(strip), tokens }));
      checkAll();
    } catch (e) {
      showError('Could not save git access', e);
    } finally {
      busy = false;
    }
  }
  async function forget(r: Row) {
    try { fill(await put<GitHostsView>('/git/hosts', { hosts: rows.map(strip), tokens: { [r.host]: null } })); } catch (e) { showError('Could not remove the token', e); }
  }

  function toggle(r: Row, p: 'https' | 'ssh') {
    r.protocols = r.protocols.includes(p) ? r.protocols.filter((x) => x !== p) : [...r.protocols, p];
  }
  function rewrite(r: Row, v: string) {
    r.preferHttps = v === 'https';
    r.preferSsh = v === 'ssh';
  }

  async function runDoctor(host: string) {
    // Read back through the store: `a ??= b` evaluates to the raw `b`, not
    // the reactive proxy, so changes to it would never show.
    doctor[host] ??= { repo: '', findings: null, running: false };
    const d = doctor[host];
    d.running = true;
    try {
      d.findings = (await post<{ findings: GitFinding[] }>('/git/doctor', { host, repo: d.repo.trim() || undefined })).findings;
    } catch (e) {
      showError(`Could not check ${host}`, e);
    } finally {
      d.running = false;
    }
  }
  async function fix(host: string, f: GitFinding) {
    if (f.fix?.kind !== 'action') return;
    try {
      if (f.id === 'ssh-host-key') {
        const fingerprint = f.fix.label.replace(/^Accept /, '');
        await post('/git/known-hosts', { host, fingerprint });
        await runDoctor(host);
      } else {
        await post('/git/login', { host });
        login = host;
      }
    } catch (e) {
      showError(f.fix.label, e);
    }
  }
  async function startLogin(host: string) {
    try { await post('/git/login', { host }); login = host; } catch (e) { showError(`Could not log in to ${host}`, e); }
  }
  // One save bar for the page: identity and hosts are saved together.
  const anyDirty = $derived(dirty || idDirty);
  async function saveAll() {
    if (idDirty) await saveIdentity();
    if (dirty) await save();
  }
  function discardAll() {
    if (view) fill(view);
    loadIdentity();
  }
  function loginEnded() {
    const h = login;
    login = null;
    if (h) runDoctor(h);
  }
</script>

{#if !view}
  <p class="empty">Loading…</p>
{:else}
  {#if !view.sandbox && view.gitconfig}
    <Callout tone="info">xherd runs natively: these settings go to <span class="mono">{view.gitconfig}</span>. Add
      <span class="mono" style="user-select:all">[include] path = {view.gitconfig}</span> to <span class="mono">~/.gitconfig</span> to use them.</Callout>
  {/if}

  <div class="sect"><h2>1 · Commit identity</h2></div>
  {#if !idName || !idEmail}
    <Callout tone="bad">Not set: git refuses to commit without a name and email.</Callout>
  {:else if idDirty}
    <Callout>Unsaved changes to the identity.</Callout>
  {:else}
    <Callout tone="ok">Agents commit as <b>{idName}</b> &lt;{idEmail}&gt;</Callout>
  {/if}
  <Panel title="Name and email" hint="Agents commit as this name and email. Without it git refuses to commit (“unable to auto-detect email address”).">
    <div class="g2">
      <label class="f">Name <input bind:value={idName} placeholder="Your Name" autocomplete="off" /></label>
      <label class="f">Email <input type="email" bind:value={idEmail} placeholder="you@example.com" autocomplete="off" /></label>
    </div>
    <div class="grp">
      <header><h3>Other identities, by repository owner</h3>
        <span class="hint">For repositories under an owner or a host, use a different identity (for example work on the same github.com). Matched on the remote URL, HTTPS or SSH.</span></header>
      {#each idRules as r, i (i)}
        <div class="idrule">
          <label class="f">Owner <input class="mono" bind:value={r.scope} placeholder="github.com/my-company" autocomplete="off" /></label>
          <label class="f">Name <input bind:value={r.name} autocomplete="off" /></label>
          <label class="f">Email <input type="email" bind:value={r.email} autocomplete="off" /></label>
          <button class="btn ghost danger sm" onclick={() => idRules.splice(i, 1)}>Remove</button>
        </div>
      {/each}
      <div><button class="btn" onclick={() => idRules.push({ scope: '', name: '', email: '' })}>+ Add an identity</button></div>
    </div>
  </Panel>

  <div class="sect"><h2>2 · Git hosts</h2><span class="hint">How agents reach each host to fetch and push; separate from the identity above.</span></div>
  {#if rows.length}
    {@const bad = rows.filter((r) => status(doctor[r.host]) === 'bad')}
    {@const pending = rows.filter((r) => ['checking', 'unknown'].includes(status(doctor[r.host])))}
    {#if bad.length}<Callout tone="bad">Access fails for {bad.map((r) => r.host).join(', ')}.</Callout>
    {:else if pending.length}<Callout glyph="…">Checking access…</Callout>
    {:else}<Callout tone="ok">Access works for {rows.map((r) => r.host).join(', ')}.</Callout>{/if}
  {:else}
    <Callout tone="bad">No host yet: add the hosts your repositories live on.</Callout>
  {/if}

  {#each rows as r (r.host)}
    {@const d = doctor[r.host]}
    <Panel label={r.host}>
      {#snippet head()}
        <h2 class="mono">{r.host}</h2>
        <span class="pill">{r.protocols.length ? r.protocols.join(' + ').toUpperCase() : 'no protocol'}</span>
        {#if status(d) === 'checking'}<span class="pill">Checking…</span>
        {:else if status(d) === 'ok'}<span class="pill live">✓ Access works</span>
        {:else if status(d) === 'bad'}<span class="pill halt">✕ {d?.findings?.find((f) => !f.ok)?.label}</span>{/if}
        <span class="grow"></span>
        <button class="btn ghost danger sm" onclick={() => (rows = rows.filter((x) => x !== r))}>Remove</button>
      {/snippet}
      <div class="row" style="gap:18px">
        {#each ['https', 'ssh'] as const as p (p)}
          <label class="f chk"><input type="checkbox" checked={r.protocols.includes(p)} onchange={() => toggle(r, p)} />{p.toUpperCase()}</label>
        {/each}
      </div>
      {#if r.protocols.includes('https')}
        <div class="g3">
          <label class="f">HTTPS token from
            <select bind:value={r.credential}>
              {#each CRED as [v, label] (v)}<option value={v}>{label}</option>{/each}
            </select></label>
          {#if r.credential === 'xherd'}
            <label class="f">Token
              <input type="password" autocomplete="off" bind:value={r.token}
                placeholder={r.hasToken ? 'stored · type to replace' : 'paste a token'} />
              {#if r.hasToken}<button class="btn ghost danger sm" style="align-self:flex-start" onclick={() => forget(r)}>Forget the stored token</button>{/if}</label>
          {/if}
          <label class="f">Username <input bind:value={r.username} placeholder="automatic" /></label>
        </div>
        <label class="f">CA bundle (optional)
          <input class="mono" bind:value={r.caFile} placeholder="/path/to/ca.pem, for a host with its own certificate authority" /></label>
        {#if r.credential === 'gh' || r.credential === 'glab'}
          <div class="row"><button class="btn" onclick={() => startLogin(r.host)}>Log in to {r.host} in the browser</button>
            <span class="hint">Shows a one-time code to enter on {r.host}.</span></div>
        {/if}
      {/if}
      <label class="f">Rewrite remotes of this host
        <select value={r.preferHttps ? 'https' : r.preferSsh ? 'ssh' : ''} onchange={(e) => rewrite(r, e.currentTarget.value)}>
          <option value="">No: use remote URLs as they are</option>
          <option value="https">SSH remotes over HTTPS</option>
          <option value="ssh">HTTPS remotes over SSH</option>
        </select></label>

      {#if login === r.host}
        <TermView path="/ws/setup/git-login" title={`Log in to ${r.host}`} height="360px" onclose={loginEnded}
          hint="Follow the steps; the check runs again when it ends." />
      {/if}

      <div class="grp">
        <header><h3>Check access</h3></header>
        <div class="row" style="flex-wrap:nowrap">
          <input class="mono" placeholder="repository URL to test (optional)"
            value={d?.repo ?? ''} oninput={(e) => { doctor[r.host] ??= { repo: '', findings: null, running: false }; doctor[r.host].repo = e.currentTarget.value; }} />
          <button class="btn" disabled={d?.running || anyDirty} title={anyDirty ? 'Save first' : ''} onclick={() => runDoctor(r.host)}>
            {d?.running ? 'Checking…' : 'Check access'}</button>
        </div>
        {#if d?.findings}
          <div class="card">
            {#each d.findings as f, i (f.id + i)}
              <div class="ck" class:first={i === 0} class:fail={!f.ok}>
                <span class="ckst {f.ok ? 'ok' : 'bad'}" aria-label={f.ok ? 'passes' : 'fails'}>{f.ok ? '✓' : '✕'}</span>
                <div class="wrap-any">
                  <b style="font-weight:500">{f.label}</b>
                  <p class="wrap-any" class:mono={f.ok} class:muted={f.ok}>{f.detail}</p>
                  {#if !f.ok && f.fix?.kind === 'command'}<div class="fix"><span>{f.fix.text}</span></div>{/if}
                </div>
                {#if !f.ok && f.fix?.kind === 'action'}
                  <button class="btn primary sm" onclick={() => fix(r.host, f)}>{f.fix.label}</button>
                {:else}<span></span>{/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </Panel>
  {/each}

  <Panel title="Add a host">
    <div class="row">
      {#each PRESETS.filter((p) => !rows.some((r) => r.host === p)) as p (p)}
        <button class="btn" onclick={() => add(p)}>+ {p}</button>
      {/each}
    </div>
    <div class="row" style="flex-wrap:nowrap">
      <input class="mono" placeholder="git.example.com" bind:value={newHost}
        onkeydown={(e) => e.key === 'Enter' && add(newHost)} />
      <button class="btn" disabled={!newHost.trim()} onclick={() => add(newHost)}>+ Add host</button>
    </div>
  </Panel>

  <SaveBar dirty={anyDirty} busy={busy || idBusy} onsave={saveAll} ondiscard={discardAll}
    saveLabel="Save git access" saved="Saved; applies right away" />
{/if}

<style>
  .idrule { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 1fr) auto; gap: 10px; align-items: end; }
  @media (max-width: 760px) {
    .idrule { grid-template-columns: minmax(0, 1fr); padding-bottom: 12px; border-bottom: 1px solid var(--fence-soft); }
    .idrule .btn { justify-self: start; }
  }
</style>
