<script lang="ts">
  // Git access per host (spec R-685, R-686): how HTTPS gets its token (gh or
  // glab login, or a token kept by xherd), SSH, optional URL rewrites, and the
  // doctor with a fix for the first failure. Used by Settings and Welcome.
  import { get, put, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';
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
  function loginEnded() {
    const h = login;
    login = null;
    if (h) runDoctor(h);
  }
</script>

{#if !view}
  <p class="muted">Loading…</p>
{:else}
  {#if !view.sandbox && view.gitconfig}
    <p class="hint">xherd runs natively: these settings go to <span class="mono">{view.gitconfig}</span>. Add
      <span class="mono" style="user-select:all">[include] path = {view.gitconfig}</span> to <span class="mono">~/.gitconfig</span> to use them.</p>
  {/if}

  <section class="sec"><div class="bd stack">
    <h3>Commit identity</h3>
    <p class="hint">Agents commit as this name and email. Without it git refuses to commit ("unable to auto-detect email address").</p>
    <div class="g3">
      <label class="f">Name <input bind:value={idName} placeholder="Your Name" autocomplete="off" /></label>
      <label class="f">Email <input type="email" bind:value={idEmail} placeholder="you@example.com" autocomplete="off" /></label>
    </div>
    <span class="lbl">Other identities, by repository owner</span>
    <p class="hint">For repositories under an owner or a host, use a different identity (for example work on the same github.com). Matched on the remote URL, HTTPS or SSH.</p>
    {#each idRules as r, i (i)}
      <div class="g3">
        <label class="f">Owner <input class="mono" bind:value={r.scope} placeholder="github.com/my-company" autocomplete="off" /></label>
        <label class="f">Name <input bind:value={r.name} autocomplete="off" /></label>
        <label class="f">Email <input type="email" bind:value={r.email} autocomplete="off" /></label>
        <div><button class="btn ghost sm" onclick={() => idRules.splice(i, 1)}>Remove</button></div>
      </div>
    {/each}
    <div><button class="btn" onclick={() => idRules.push({ scope: '', name: '', email: '' })}>+ Add an identity</button></div>
    <div><button class="btn" disabled={idBusy || !idDirty} onclick={saveIdentity}>{idBusy ? 'Saving…' : 'Save identity'}</button>
      {#if !idDirty && idName && idEmail}<span class="hint">Saved; applies to new commits at once.</span>{/if}</div>
  </div></section>

  {#each rows as r (r.host)}
    {@const d = doctor[r.host]}
    <section class="sec">
      <div class="sh" style="flex-direction:row;align-items:center;gap:10px;flex-wrap:wrap"><h2 class="mono">{r.host}</h2>
        <span class="hint">{r.protocols.length ? r.protocols.join(' + ').toUpperCase() : 'no protocol'}</span>
        {#if status(d) === 'checking'}<span class="hint">Checking…</span>
        {:else if status(d) === 'ok'}<span class="ckst ok" aria-label="access works">✓</span><span class="hint">Access works</span>
        {:else if status(d) === 'bad'}<span class="ckst bad" aria-label="access fails">✕</span><span class="hint">{d?.findings?.find((f) => !f.ok)?.label}</span>{/if}
        <span class="grow"></span>
        <button class="btn ghost sm" onclick={() => (rows = rows.filter((x) => x !== r))}>Remove</button></div>
      <div class="bd">
        <div class="row" style="gap:18px">
          {#each ['https', 'ssh'] as const as p (p)}
            <label class="row" style="gap:6px;width:auto"><input type="checkbox" style="width:auto;height:auto"
              checked={r.protocols.includes(p)} onchange={() => toggle(r, p)} />{p.toUpperCase()}</label>
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
                {#if r.hasToken}<button class="btn ghost sm" style="width:fit-content" onclick={() => forget(r)}>Forget the stored token</button>{/if}</label>
            {/if}
            <label class="f">Username <input bind:value={r.username} placeholder="automatic" /></label>
          </div>
          <label class="f">CA bundle (optional)
            <input class="mono" bind:value={r.caFile} placeholder="/path/to/ca.pem, for a host with its own certificate authority" /></label>
          {#if r.credential === 'gh' || r.credential === 'glab'}
            <div><button class="btn" onclick={() => startLogin(r.host)}>Log in to {r.host} in the browser</button>
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

        <div class="row" style="gap:8px;border-top:1px solid var(--fence-soft);padding-top:12px">
          <input class="mono" style="max-width:420px" placeholder="repository URL to test (optional)"
            value={d?.repo ?? ''} oninput={(e) => { doctor[r.host] ??= { repo: '', findings: null, running: false }; doctor[r.host].repo = e.currentTarget.value; }} />
          <button class="btn" disabled={d?.running || dirty} title={dirty ? 'Save first' : ''} onclick={() => runDoctor(r.host)}>
            {d?.running ? 'Checking…' : 'Check access'}</button>
        </div>
        {#if d?.findings}
          <div class="card">
            {#each d.findings as f, i (f.id + i)}
              <div class="ck" class:first={i === 0} style={f.ok ? '' : 'background:var(--halt-wash)'}>
                <span class="ckst {f.ok ? 'ok' : 'bad'}" aria-label={f.ok ? 'passes' : 'fails'}>{f.ok ? '✓' : '✕'}</span>
                <div style="min-width:0">
                  <b style="font-weight:500">{f.label}</b>
                  <p class={f.ok ? 'mono muted' : ''} style={f.ok ? '' : 'color:var(--halt);word-break:break-word'}>{f.detail}</p>
                  {#if !f.ok && f.fix?.kind === 'command'}<div class="fix"><span style="user-select:all">{f.fix.text}</span></div>{/if}
                </div>
                {#if !f.ok && f.fix?.kind === 'action'}
                  <button class="btn primary sm" onclick={() => fix(r.host, f)}>{f.fix.label}</button>
                {:else}<span></span>{/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </section>
  {/each}

  <div class="row" style="gap:8px">
    {#each PRESETS.filter((p) => !rows.some((r) => r.host === p)) as p (p)}
      <button class="btn" onclick={() => add(p)}>+ {p}</button>
    {/each}
    <input class="mono" style="max-width:240px" placeholder="git.example.com" bind:value={newHost}
      onkeydown={(e) => e.key === 'Enter' && add(newHost)} />
    <button class="btn" disabled={!newHost.trim()} onclick={() => add(newHost)}>+ Add host</button>
  </div>

  <div class="savebar">
    <span class="muted">{dirty ? 'Unsaved changes' : 'Saved; applies right away'}</span>
    <span class="grow"></span>
    <button class="btn ghost" disabled={!dirty || busy} onclick={() => view && fill(view)}>Discard</button>
    <button class="btn primary" disabled={!dirty || busy} onclick={save}>Save git access</button>
  </div>
{/if}
