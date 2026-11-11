<script lang="ts">
  // Settings: General, Gates & autonomy, Git access and Plugins. Saved to
  // config.json; most apply at once.
  import { app } from '../lib/store.svelte';
  import { put } from '../lib/api';
  import { route, href } from '../lib/router.svelte';
  import { setup, loadConfig } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import GitAccess from '../components/GitAccess.svelte';
  import Backups from '../components/Backups.svelte';
  import Profiles from '../components/Profiles.svelte';
  import Toolchains from '../components/Toolchains.svelte';
  import type { ConfigView, PluginInfo } from '../lib/types';

  const TABS = ['general', 'gates', 'notifications', 'git', 'profiles', 'backups', 'plugins', 'about'];
  const tab = $derived(TABS.find((t) => t === route.query.get('tab')) ?? 'general');
  const broken = $derived(app.state?.brokenPlugins ?? []);
  const open = $derived((app.state?.plugins ?? []).find((p) => p.name === route.query.get('plugin')));

  type Form = Record<string, string | boolean>;
  const FIELDS = ['maxConcurrentAgents', 'pauseTimeoutSec', 'workspace', 'eventRetentionDays', 'idleAlertMinutes', 'gateParkMinutes', 'freshSessions', 'autonomy', 'maxAutoApprovalsInRow', 'ntfyServer', 'ntfyTopic', 'mutedNotifications', 'desktopNotifications'];
  let form = $state<Form>({});
  const KINDS: [string, string][] = [['gate', 'A gate needs your approval'], ['blocked', 'An agent is blocked'], ['idle', 'An agent went idle'], ['delivery', 'A message was not acknowledged'], ['error', 'An agent failed'], ['recovery', 'An agent was recovered'], ['budget', 'Token budget'], ['limit', 'A limit was reached'], ['auto-approved', 'Something was approved for you'], ['pr', 'Pull request news'], ['pr-comments', 'New PR comments'], ['todo', 'A TODO came due']];
  const muted = $derived(new Set(String(form.mutedNotifications ?? '').split(',').filter(Boolean)));
  function toggleKind(k: string, on: boolean) {
    const next = new Set(muted);
    if (on) next.delete(k); else next.add(k);
    form.mutedNotifications = [...next].sort().join(',');
  }
  let plugins = $state<Record<string, Record<string, string>>>({});
  let saving = $state(false);
  let ready = $state(false);

  function fill(v: ConfigView) {
    const c = v.config;
    form = Object.fromEntries(FIELDS.map((k) => [k, typeof c[k] === 'boolean' ? (c[k] as boolean) : String(c[k] ?? '')]));
    const ps = (c.pluginSettings ?? {}) as Record<string, Record<string, unknown>>;
    plugins = Object.fromEntries(
      (app.state?.plugins ?? []).map((p) => [p.name, Object.fromEntries(p.settings.filter((s) => !s.secret).map((s) => [s.key, String(ps[p.name]?.[s.key] ?? '')]))]),
    );
    ready = true;
  }
  $effect(() => { if (setup.config && app.state && !ready) fill(setup.config); });
  $effect(() => { if (!setup.config) loadConfig().catch((e) => showError('Could not load settings', e)); });

  const saved = $derived.by(() => {
    if (!setup.config) return {} as Form;
    const c = setup.config.config;
    return Object.fromEntries(FIELDS.map((k) => [k, typeof c[k] === 'boolean' ? (c[k] as boolean) : String(c[k] ?? '')])) as Form;
  });
  const changed = $derived(FIELDS.filter((k) => form[k] !== undefined && form[k] !== saved[k]));
  const savedPlugins = (p: string, k: string) => String(((setup.config?.config.pluginSettings ?? {}) as Record<string, Record<string, unknown>>)[p]?.[k] ?? '');
  const changedPlugins = $derived(
    Object.entries(plugins).filter(([p, kv]) => Object.entries(kv).some(([k, v]) => v !== savedPlugins(p, k))).map(([p]) => p),
  );
  const dirty = $derived(changed.length + changedPlugins.length);

  async function save() {
    saving = true;
    const body: Record<string, unknown> = {};
    for (const k of changed) body[k] = typeof form[k] === 'boolean' ? form[k] : k === 'mutedNotifications' ? String(form[k]).split(',').filter(Boolean) : k === 'workspace' || k === 'autonomy' || k === 'ntfyServer' || k === 'ntfyTopic' ? form[k] : Number(form[k]);
    if (changedPlugins.length) {
      body.pluginSettings = Object.fromEntries(
        changedPlugins.map((p) => [p, Object.fromEntries(Object.entries(plugins[p]).filter(([, v]) => v !== ''))]),
      );
    }
    try {
      setup.config = await put<ConfigView>('/config', body);
      fill(setup.config);
    } catch (e) {
      showError('Could not save settings', e);
    } finally {
      saving = false;
    }
  }
  const initials = (p: PluginInfo) => (p.label || p.name).split(/[\s-]+/).map((w) => w[0]).join('').slice(0, 2).toUpperCase();
  const HUE: Record<string, string> = { runtime: '#b4441f', host: '#4a3a75', tracker: '#2f45c9' };
</script>

<div class="phead">
  <div style="display:flex;flex-direction:column;gap:4px">
    <h1>Settings</h1>
    <p class="muted">How many agents run at once, how long xherd waits for you, git access, and the plugins it uses.</p>
  </div>
  <nav class="tabs" aria-label="Settings">
    <a class="tb" class:on={tab === 'general'} aria-current={tab === 'general' ? 'page' : undefined} href={href('/settings')}>General</a>
    <a class="tb" class:on={tab === 'gates'} aria-current={tab === 'gates' ? 'page' : undefined} href={href('/settings', { tab: 'gates' })}>Gates &amp; autonomy</a>
    <a class="tb" class:on={tab === 'notifications'} aria-current={tab === 'notifications' ? 'page' : undefined} href={href('/settings', { tab: 'notifications' })}>Notifications</a>
    <a class="tb" class:on={tab === 'git'} aria-current={tab === 'git' ? 'page' : undefined} href={href('/settings', { tab: 'git' })}>Git access</a>
    <a class="tb" class:on={tab === 'profiles'} aria-current={tab === 'profiles' ? 'page' : undefined} href={href('/settings', { tab: 'profiles' })}>Repo profiles</a>
    <a class="tb" class:on={tab === 'backups'} aria-current={tab === 'backups' ? 'page' : undefined} href={href('/settings', { tab: 'backups' })}>Backups</a>
    <a class="tb" class:on={tab === 'plugins'} aria-current={tab === 'plugins' ? 'page' : undefined} href={href('/settings', { tab: 'plugins' })}>
      Plugins{#if broken.length}<span class="tc h">{broken.length} error{broken.length > 1 ? 's' : ''}</span>{/if}</a>
    <a class="tb" class:on={tab === 'about'} aria-current={tab === 'about' ? 'page' : undefined} href={href('/settings', { tab: 'about' })}>About</a>
  </nav>
</div>

<div class="stack">
  {#if setup.config?.restartNeeded}
    <div class="savebar" role="status" style="border-color:var(--accent)">
      <span>Saved settings differ from the running daemon. <b>Restart the daemon</b> to apply them.</span>
    </div>
  {/if}
  {#if tab === 'git'}
    <GitAccess />
  {:else if tab === 'profiles'}
    <Profiles />
  {:else if tab === 'backups'}
    <Backups />
  {:else if tab === 'about'}
    <section class="sec"><div class="sh"><h2>About xherd</h2></div><div class="bd">
      <dl class="kv">
        <dt>Version</dt><dd class="mono">{app.state?.buildVersion}</dd>
        <dt>Runs</dt><dd>{#if app.state?.sandbox}In the Docker container{:else}Natively on {app.state?.platform}{/if}</dd>
        <dt>License</dt><dd>MIT</dd>
      </dl>
      <p class="hint">A release shows its tag (<span class="mono">0.2.0</span>); a development build shows the last tag, the commits since it and the commit hash (<span class="mono">0.2.0+3.abc1234</span>), with <span class="mono">dirty</span> for uncommitted changes.</p>
    </div></section>
  {:else if !setup.config || !ready}
    <p class="muted">Loading…</p>
  {:else if tab === 'general'}
    <div class="secs">
      <section class="sec">
        <div class="sh"><h2>Agents</h2><span class="hint">How many run at once.</span></div>
        <div class="bd">
          <label class="f">Max agents running at once
            <span class="unit"><input type="number" min="1" max="64" bind:value={form.maxConcurrentAgents} /><span>agents</span></span>
            <span class="hint">More wait in Queued.</span></label>
          <label class="f">Pause timeout
            <span class="unit"><input type="number" min="5" max="3600" bind:value={form.pauseTimeoutSec} /><span>sec</span></span>
            <span class="hint">How long a pause waits for the agent to stop.</span></label>
        </div>
      </section>
      <section class="sec">
        <div class="sh"><h2>Files and history</h2><span class="hint">Where work lives and how long events are kept.</span></div>
        <div class="bd">
          <label class="f">Workspace folder
            <input class="mono" bind:value={form.workspace} placeholder="$XHERD_WORKSPACE, else ~/xherd" />
            <span class="hint">Holds <span class="mono">worktrees/&lt;repo&gt;/&lt;agent&gt;</span>. Absolute path; empty for the default.</span></label>
          <label class="f">Keep events
            <span class="unit"><input type="number" min="1" bind:value={form.eventRetentionDays} /><span>days</span></span></label>
        </div>
      </section>
    </div>
  {:else if tab === 'gates'}
    <div class="secs">
      <section class="sec">
        <div class="sh"><h2>Waiting at a gate</h2><span class="hint">What happens while an agent sits at a gate.</span></div>
        <div class="bd">
          <label class="f">Idle alert after
            <span class="unit"><input type="number" min="0" bind:value={form.idleAlertMinutes} /><span>min</span></span>
            <span class="hint">0 turns it off.</span></label>
          <label class="f">Park at a gate after
            <span class="unit"><input type="number" min="0" bind:value={form.gateParkMinutes} /><span>min</span></span>
            <span class="hint">Frees the slot; stays in Needs you. 0 never parks.</span></label>
          {#each [['freshSessions', 'Fresh session after each approved gate', 'The next step starts a new session that reloads everything from .xherd/.']] as [k, title, hint] (k)}
            <div class="tog">
              <button class="sw" class:on={form[k] === true} role="switch" aria-checked={form[k] === true} aria-label={title}
                onclick={() => (form[k] = !form[k])}></button>
              <div><b style="font-weight:500">{title}</b><div class="hint">{hint}</div></div>
            </div>
          {/each}
        </div>
      </section>
      <section class="sec">
        <div class="sh"><h2>Autonomy</h2><span class="hint">What xherd may approve without you.</span></div>
        <div class="bd">
          <label class="f">Autonomy of new agents
            <select bind:value={form.autonomy}><option value="ask">ask</option><option value="important">important</option></select>
            <span class="hint">important: xherd approves design, tasks, review and commit gates when no tripwire holds them. Spec and ship always wait. Each agent can change it.</span></label>
          <label class="f">Stop auto-approving after
            <span class="unit"><input type="number" min="0" bind:value={form.maxAutoApprovalsInRow} /><span>in a row</span></span>
            <span class="hint">The next gate waits for you. 0 = no limit.</span></label>
        </div>
      </section>
    </div>
  {:else if tab === 'notifications'}
    <div class="secs">
      <section class="sec">
        <div class="sh"><h2>In this browser</h2><span class="hint">The bell in the top bar lists what happened while xherd was open.</span></div>
        <div class="bd">
          <p class="muted">Turn browser notifications on from the bell; the browser asks for permission then. Each agent has one per kind (gate, blocked, idle…); a new one replaces the previous.</p>
        </div>
      </section>
      <section class="sec">
        <div class="sh"><h2>Which ones</h2><span class="hint">Untick a kind to stop receiving it anywhere (bell, browser, ntfy, desktop). Applies at once.</span></div>
        <div class="bd">
          {#each KINDS as [k, label] (k)}
            <label class="f chk"><input type="checkbox" checked={!muted.has(k)} onchange={(e) => toggleKind(k, e.currentTarget.checked)} /> {label}</label>
          {/each}
        </div>
      </section>
      <section class="sec">
        <div class="sh"><h2>Desktop</h2><span class="hint">System notifications from the daemon itself, also when no browser tab is open.</span></div>
        <div class="bd">
          <label class="f chk"><input type="checkbox" checked={form.desktopNotifications === true} onchange={(e) => (form.desktopNotifications = e.currentTarget.checked)} /> Show system notifications</label>
          <span class="hint">Uses <code>osascript</code> (macOS) or <code>notify-send</code> (Linux) on the machine running the daemon. Not available in the Docker image: there, use the browser or ntfy.</span>
        </div>
      </section>
      <section class="sec">
        <div class="sh"><h2>ntfy</h2><span class="hint">Also send them to your phone through an ntfy topic.</span></div>
        <div class="bd">
          <label class="f">Server
            <input class="mono" bind:value={form.ntfyServer} placeholder="https://ntfy.example.com" />
            <span class="hint">Off until server and topic are both set. Messages carry agent names and reasons (never PR comments or documents) and go to this server, in plain text: use your own, or treat the topic as a password.</span></label>
          <label class="f">Topic
            <input class="mono" bind:value={form.ntfyTopic} placeholder="xherd-your-secret-name" />
            <span class="hint">Letters, digits, - and _. Applies at once, no restart.</span></label>
        </div>
      </section>
    </div>
  {:else}
    {#if tab === 'plugins' && open}
      <a class="hint" href={href('/settings', { tab: 'plugins' })}>← All plugins</a>
      <section class="sec">
        <div class="sh"><h2>{open.label || open.name}</h2></div>
        <div class="bd">
          <div class="hint">{open.kind} · {open.version}{open.builtin ? ' · built in' : ''} · {open.enabled ? 'on' : 'off'}</div>
          {#if open.description}<p class="muted">{open.description}</p>{/if}
          {#each open.settings as s (s.key)}
            {#if s.secret}
              <div class="f">{s.label || s.key}<span class="hint">Secret: kept in secrets.json, not editable here yet.</span></div>
            {:else if plugins[open.name]}
              <label class="f">{s.label || s.key}{#if s.required} *{/if}
                <input bind:value={plugins[open.name][s.key]} placeholder={s.default != null ? String(s.default) : ''} /></label>
            {/if}
          {:else}
            <p class="hint">This plugin has no settings.</p>
          {/each}
        </div>
      </section>
      {#if open.kind === 'toolchain'}
        {#key open.name}<Toolchains plugin={open.name} />{/key}
      {/if}
    {:else}
    {#snippet pcard(p: PluginInfo)}
        <a class="pcard" href={href('/settings', { tab: 'plugins', plugin: p.name })} style="color:inherit;text-decoration:none">
          <div class="h">
            <span class="mi" style="background:{HUE[p.kind] ?? '#5b4b80'}" aria-hidden="true">{initials(p)}</span>
            <div style="min-width:0">
              <h3>{p.label || p.name}</h3>
              <div class="hint">{p.kind} · {p.version}{p.builtin ? ' · built in' : ''}</div>
            </div>
          </div>
          <div class="b">
            {#if p.description}<p class="muted">{p.description}</p>{/if}
            <span class="hint">{p.settings.length ? `${p.settings.length} setting${p.settings.length > 1 ? 's' : ''}` : 'No settings'}{p.kind === 'toolchain' ? ' · tools and versions' : ''}</span>
          </div>
          <div class="ft">{#if p.enabled}<span class="pill live">● on</span>{:else}<span class="pill">○ off</span>{/if}</div>
        </a>
    {/snippet}
    {@const plugs = app.state?.plugins ?? []}
    <h2 class="hint" style="margin:0 0 8px">Enabled</h2>
    <div class="cards">
      {#each plugs.filter((p) => p.enabled) as p (p.name)}{@render pcard(p)}{/each}
    </div>
    {#if plugs.some((p) => !p.enabled)}
      <h2 class="hint" style="margin:16px 0 8px">Available</h2>
      <p class="hint" style="margin:0 0 8px">Installed but switched off. They can be switched on in the Welcome page tools step.</p>
      <div class="cards">
        {#each plugs.filter((p) => !p.enabled) as p (p.name)}{@render pcard(p)}{/each}
      </div>
    {/if}
    {#if broken.length}
    <div class="cards">
      {#each broken as b (b.dir)}
        <article class="pcard err">
          <div class="h"><span class="mi" style="background:var(--halt)">!</span>
            <div><h3>{b.name}</h3><div class="hint mono">{b.dir}</div></div></div>
          <div class="b"><p style="color:var(--halt);word-break:break-word">{b.error}</p></div>
        </article>
      {/each}
    </div>
    {/if}
    {/if}
  {/if}
  {#if setup.config && ready && tab !== 'git' && tab !== 'backups' && tab !== 'profiles' && tab !== 'about'}
    <div class="savebar">
      <span class="muted">{dirty ? `${dirty} unsaved change${dirty > 1 ? 's' : ''}` : 'No unsaved changes'}</span>
      <span class="grow"></span>
      <button class="btn ghost" disabled={!dirty || saving} onclick={() => setup.config && fill(setup.config)}>Discard</button>
      <button class="btn primary" disabled={!dirty || saving} onclick={save}>Save changes</button>
    </div>
  {/if}
</div>
