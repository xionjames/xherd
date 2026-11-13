<script lang="ts">
  // Git access per host (spec R-685, R-686): how HTTPS gets its token (gh or
  // glab login, or a token kept by xherd), SSH, optional URL rewrites, and the
  // doctor with a fix for the first failure. Used by Settings and Welcome (the commit identity is GitIdentity.svelte).
  // Rows with `from` are implicit: a Git provider plugin that is on covers that
  // host; they are never saved, and Edit turns one into an ordinary row.
  import { get, put, post } from '../lib/api';
  import { app } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';
  import Panel from './Panel.svelte';
  import SaveBar from './SaveBar.svelte';
  import Callout from './Callout.svelte';
  import GitconfigNote from './GitconfigNote.svelte';
  import { claims, boundProvider } from '../lib/gitProviders';
  import type { GitHost, GitHostsView, GitFinding, GitProvider } from '../lib/types';

  // `heading` off where the page already has this title (Welcome); `note` off
  // where the identity above shows the gitconfig note already (Settings).
  // `bar` off in Welcome, whose single button saves through `save()`; `unsaved`
  // is read back by the page. `pluginsHref` is where Git provider plugins are
  // switched on; `onlogin` runs after a browser login ends (Welcome reloads
  // its checks).
  let { heading = true, note = true, bar = true, unsaved = $bindable(false),
    pluginsHref = '#/settings?tab=plugins', onlogin }: {
    heading?: boolean; note?: boolean; bar?: boolean; unsaved?: boolean;
    pluginsHref?: string; onlogin?: () => void;
  } = $props();

  type Row = GitHost & { token: string };
  let view = $state<GitHostsView | null>(null);
  let rows = $state<Row[]>([]);
  let saved = $state('');
  let busy = $state(false);
  let newHost = $state('');
  let doctor = $state<Record<string, { repo: string; findings: GitFinding[] | null; running: boolean }>>({});
  let login = $state<string | null>(null);

  // The provider a host is bound to: the one named, else the first claiming
  // its name (as the daemon does).
  const providers = $derived(view?.providers ?? []);
  const claimedBy = (host: string) => providers.find((p) => claims(p, host));
  const providerOf = (r: Row) => boundProvider(providers, r);
  // A provider with no credential helper cannot be the token's source.
  function bind(r: Row) {
    if (r.credential === 'provider' && !providerOf(r)?.helper) r.credential = 'xherd';
  }
  const credOptions = (r: Row): [GitHost['credential'], string][] => {
    const p = providerOf(r);
    return [
      ...(p?.helper ? [['provider', `${p.name}'s login`] as [GitHost['credential'], string]] : []),
      ['xherd', 'A token kept by xherd'],
      ['none', 'git decides (your own config)'],
    ];
  };
  const PRESETS = ['github.com', 'gitlab.com', 'bitbucket.org'];

  const strip = (r: Row): GitHost => {
    const { token: _t, hasToken: _h, from: _f, ...h } = r;
    return { ...h, provider: h.provider || undefined, username: h.username || undefined, caFile: h.caFile || undefined };
  };
  // What is saved: the user's rows, never the implicit ones.
  const own = () => rows.filter((r) => !r.from).map(strip);
  const snapshot = () => JSON.stringify(own());
  const pluginLabel = (name: string) => app.state?.plugins.find((p) => p.name === name)?.label ?? name;
  const dirty = $derived(snapshot() !== saved || rows.some((r) => r.token !== ''));
  $effect(() => { unsaved = dirty; });
  export async function saveAll() { if (dirty) await save(); return !dirty; }

  function fill(v: GitHostsView) {
    view = v;
    rows = v.hosts.map((h) => ({ ...h, provider: h.provider ?? '', username: h.username ?? '', caFile: h.caFile ?? '', token: '' }));
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

  function add(host: string) {
    host = host.trim().toLowerCase();
    if (!host || rows.some((r) => r.host === host)) return;
    const p = providers.find((p) => claims(p, host));
    const credential: GitHost['credential'] = p?.helper ? 'provider' : 'xherd';
    rows.push({ host, protocols: ['https', 'ssh'], credential, username: '', caFile: '', preferHttps: false, preferSsh: false, token: '' });
    newHost = '';
  }

  async function save() {
    busy = true;
    const tokens = Object.fromEntries(rows.filter((r) => r.token !== '').map((r) => [r.host, r.token]));
    try {
      fill(await put<GitHostsView>('/git/hosts', { hosts: own(), tokens }));
      checkAll();
    } catch (e) {
      showError('Could not save git access', e);
    } finally {
      busy = false;
    }
  }
  async function forget(r: Row) {
    try { fill(await put<GitHostsView>('/git/hosts', { hosts: own(), tokens: { [r.host]: null } })); } catch (e) { showError('Could not remove the token', e); }
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
    onlogin?.();
  }
</script>

{#if !view}
  <p class="empty">Loading…</p>
{:else}
  {#if note}<GitconfigNote />{/if}

  {#if heading}<div class="sect"><h2>Git hosts</h2><span class="hint">How agents reach each host to fetch and push.</span></div>{/if}
  {#if rows.length}
    {@const bad = rows.filter((r) => status(doctor[r.host]) === 'bad')}
    {@const pending = rows.filter((r) => ['checking', 'unknown'].includes(status(doctor[r.host])))}
    {#if bad.length}<Callout tone="bad">Access fails for {bad.map((r) => r.host).join(', ')}.</Callout>
    {:else if pending.length}<Callout glyph="…">Checking access…</Callout>
    {:else}<Callout tone="ok">Access works for {rows.map((r) => r.host).join(', ')}.</Callout>{/if}
  {:else if !providers.length}
    <Callout>Configure access by hand below (SSH key, token), or
      <a href={pluginsHref}>switch on a Git provider plugin</a> to do it for you.</Callout>
  {:else}
    <Callout tone="bad">No host yet: add the hosts your repositories live on.</Callout>
  {/if}

  {#each rows as r (r.host)}
    {@const d = doctor[r.host]}
    <Panel label={r.host}>
      {#snippet head()}
        <h2 class="mono">{r.host}</h2>
        {#if r.from}<span class="pill">From the {pluginLabel(r.from)} plugin</span>{/if}
        <span class="pill">{r.protocols.length ? r.protocols.join(' + ').toUpperCase() : 'no protocol'}</span>
        {#if status(d) === 'checking'}<span class="pill">Checking…</span>
        {:else if status(d) === 'ok'}<span class="pill live">✓ Access works</span>
        {:else if status(d) === 'bad'}<span class="pill halt">✕ {d?.findings?.find((f) => !f.ok)?.label}</span>{/if}
        <span class="grow"></span>
        {#if r.from}
          <button class="btn ghost sm" title="Make this host your own row, to change it" onclick={() => (r.from = undefined)}>Edit</button>
        {:else}
          <button class="btn ghost danger sm" onclick={() => (rows = rows.filter((x) => x !== r))}>Remove</button>
        {/if}
      {/snippet}
      {#if r.from}
        <p class="hint">The {pluginLabel(r.from)} plugin gives git its login for {r.host} over HTTPS while it is on. Edit to change how this host is reached.</p>
        {#if providerOf(r)?.login}
          <div class="row"><button class="btn" onclick={() => startLogin(r.host)}>Log in to {r.host} in the browser</button>
            <span class="hint">Shows a one-time code to enter on {r.host}.</span></div>
        {/if}
      {:else}
      <div class="row" style="gap:18px">
        {#each ['https', 'ssh'] as const as p (p)}
          <label class="f chk"><input type="checkbox" checked={r.protocols.includes(p)} onchange={() => toggle(r, p)} />{p.toUpperCase()}</label>
        {/each}
      </div>
      {#if r.protocols.includes('https')}
        <div class="g3">
          <label class="f">Provider
            <select bind:value={r.provider} onchange={() => bind(r)}>
              <option value="">{claimedBy(r.host) ? `Automatic (${claimedBy(r.host)?.name})` : 'None'}</option>
              {#each providers as p (p.name)}<option value={p.name}>{p.name}</option>{/each}
            </select>
            {#if providerOf(r)}<a class="hint" href={`#/settings?tab=plugins&plugin=${providerOf(r)?.name}`}>{providerOf(r)?.name} plugin options</a>{/if}</label>
          <label class="f">HTTPS token from
            <select bind:value={r.credential}>
              {#each credOptions(r) as [v, label] (v)}<option value={v}>{label}</option>{/each}
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
        {#if r.credential === 'provider' && providerOf(r)?.login}
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
      {/if}

      {#if login === r.host}
        <TermView path="/ws/setup/git-login" title={`Log in to ${r.host}`} height="360px" onclose={loginEnded}
          hint="Follow the steps; the check runs again when it ends." />
      {/if}

      <div class="grp">
        <header><h3>Check access</h3></header>
        <div class="row" style="flex-wrap:nowrap">
          <input class="mono" placeholder="repository URL to test (optional)"
            value={d?.repo ?? ''} oninput={(e) => { doctor[r.host] ??= { repo: '', findings: null, running: false }; doctor[r.host].repo = e.currentTarget.value; }} />
          <button class="btn" disabled={d?.running || dirty} title={dirty ? 'Save first' : ''} onclick={() => runDoctor(r.host)}>
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

  {#if bar}<SaveBar {dirty} {busy} onsave={save} ondiscard={() => view && fill(view)}
    saveLabel="Save git access" saved="Saved; applies right away" />{/if}
{/if}

<style>
</style>
