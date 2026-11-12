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
  import OpenIn from '../components/OpenIn.svelte';
  import Toolchains from '../components/Toolchains.svelte';
  import PageHead from '../components/PageHead.svelte';
  import Panel from '../components/Panel.svelte';
  import Tabs from '../components/Tabs.svelte';
  import SaveBar from '../components/SaveBar.svelte';
  import Callout from '../components/Callout.svelte';
  import type { ConfigView, PluginInfo } from '../lib/types';
  import { MODE_HINT, modes, type PermissionMode } from '../lib/modes';

  const TABS = ['general', 'gates', 'notifications', 'openin', 'git', 'profiles', 'backups', 'plugins', 'about'];
  const tab = $derived(TABS.find((t) => t === route.query.get('tab')) ?? 'general');
  const broken = $derived(app.state?.brokenPlugins ?? []);
  const open = $derived((app.state?.plugins ?? []).find((p) => p.name === route.query.get('plugin')));

  type Form = Record<string, string | boolean>;
  const FIELDS = ['maxConcurrentAgents', 'pauseTimeoutSec', 'workspace', 'eventRetentionDays', 'idleAlertMinutes', 'gateParkMinutes', 'freshSessions', 'autonomy', 'permissionMode', 'maxAutoApprovalsInRow', 'ntfyServer', 'ntfyTopic', 'mutedNotifications', 'desktopNotifications'];
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
    for (const k of changed) body[k] = typeof form[k] === 'boolean' ? form[k] : k === 'mutedNotifications' ? String(form[k]).split(',').filter(Boolean) : k === 'workspace' || k === 'autonomy' || k === 'permissionMode' || k === 'ntfyServer' || k === 'ntfyTopic' ? form[k] : Number(form[k]);
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
  const TAB_LABEL: Record<string, string> = { general: 'General', gates: 'Gates & autonomy', notifications: 'Notifications', openin: 'Open in', git: 'Git access', profiles: 'Repo profiles', backups: 'Backups', plugins: 'Plugins', about: 'About' };
  const tabItems = $derived(TABS.map((t) => ({
    id: t, label: TAB_LABEL[t], href: href('/settings', t === 'general' ? {} : { tab: t }),
    badge: t === 'plugins' && broken.length ? { n: `${broken.length} error${broken.length > 1 ? 's' : ''}`, tone: 'h' as const } : undefined,
  })));
</script>

<PageHead title="Settings" sub="How many agents run at once, how long xherd waits for you, git access, and the plugins it uses.">
  {#snippet tabs()}<Tabs label="Settings" items={tabItems} current={tab} />{/snippet}
</PageHead>

<div class="stack mid">
  {#if setup.config?.restartNeeded}
    <Callout tone="info">Saved settings differ from the running daemon. <b>Restart the daemon</b> to apply them.</Callout>
  {/if}
  {#if tab === 'git'}
    <GitAccess />
  {:else if tab === 'openin'}
    <OpenIn />
  {:else if tab === 'profiles'}
    <Profiles />
  {:else if tab === 'backups'}
    <Backups />
  {:else if tab === 'about'}
    <Panel title="About xherd">
      <dl class="kv">
        <dt>Version</dt><dd class="mono">{app.state?.buildVersion}</dd>
        <dt>Runs</dt><dd>{#if app.state?.sandbox}In the Docker container{:else}Natively on {app.state?.platform}{/if}</dd>
        <dt>License</dt><dd>MIT</dd>
      </dl>
      <p class="hint">A release shows its tag (<span class="mono">0.2.0</span>); a development build shows the last tag, the commits since it and the commit hash (<span class="mono">0.2.0+3.abc1234</span>), with <span class="mono">dirty</span> for uncommitted changes.</p>
    </Panel>
  {:else if !setup.config || !ready}
    <p class="empty">Loading…</p>
  {:else if tab === 'general'}
    <div class="secs">
      <Panel title="Agents" hint="How many run at once.">
        <label class="f">Max agents running at once
          <span class="unit"><input type="number" min="1" max="64" bind:value={form.maxConcurrentAgents} /><span>agents</span></span>
          <span class="hint">More wait in Queued.</span></label>
        <label class="f">Pause timeout
          <span class="unit"><input type="number" min="5" max="3600" bind:value={form.pauseTimeoutSec} /><span>sec</span></span>
          <span class="hint">How long a pause waits for the agent to stop.</span></label>
      </Panel>
      <Panel title="Files and history" hint="Where work lives and how long events are kept.">
        <label class="f">Workspace folder
          <input class="mono" bind:value={form.workspace} placeholder="$XHERD_WORKSPACE, else ~/xherd" />
          <span class="hint">Holds <span class="mono">worktrees/&lt;repo&gt;/&lt;agent&gt;</span>. Absolute path; empty for the default.</span></label>
        <label class="f">Keep events
          <span class="unit"><input type="number" min="1" bind:value={form.eventRetentionDays} /><span>days</span></span></label>
      </Panel>
    </div>
  {:else if tab === 'gates'}
    <div class="secs">
      <Panel title="Waiting at a gate" hint="What happens while an agent sits at a gate.">
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
      </Panel>
      <Panel title="Autonomy" hint="What xherd may approve without you.">
        <label class="f">Autonomy of new agents
          <select bind:value={form.autonomy}><option value="ask">ask</option><option value="important">important</option></select>
          <span class="hint">important: xherd approves design, tasks, review and commit gates when no tripwire holds them. Spec and ship always wait. Each agent can change it.</span></label>
        <label class="f">Stop auto-approving after
          <span class="unit"><input type="number" min="0" bind:value={form.maxAutoApprovalsInRow} /><span>in a row</span></span>
          <span class="hint">The next gate waits for you. 0 = no limit.</span></label>
      </Panel>
      <Panel title="Tool permissions" hint="What an agent's runtime may do without asking you.">
        <label class="f">Tool permissions of new agents
          <select bind:value={form.permissionMode}>{#each modes(!!app.state?.sandbox) as m (m)}<option value={m}>{m}</option>{/each}</select>
          <span class="hint">{MODE_HINT[(form.permissionMode as PermissionMode) ?? 'edits']} A repo profile or the New agent form can choose another; each agent can change it.</span></label>
      </Panel>
    </div>
  {:else if tab === 'notifications'}
    <div class="secs">
      <Panel title="In this browser" hint="The bell in the top bar lists what happened while xherd was open.">
        <p class="muted">Turn browser notifications on from the bell; the browser asks for permission then. Each agent has one per kind (gate, blocked, idle…); a new one replaces the previous.</p>
      </Panel>
      <Panel title="Which ones" hint="Untick a kind to stop receiving it anywhere (bell, browser, ntfy, desktop).">
        <div class="vlist">
          {#each KINDS as [k, label] (k)}
            <label class="f chk"><input type="checkbox" checked={!muted.has(k)} onchange={(e) => toggleKind(k, e.currentTarget.checked)} /> {label}</label>
          {/each}
        </div>
      </Panel>
      <Panel title="Desktop" hint="System notifications from the daemon itself, also when no browser tab is open.">
        <label class="f chk"><input type="checkbox" checked={form.desktopNotifications === true} onchange={(e) => (form.desktopNotifications = e.currentTarget.checked)} /> Show system notifications</label>
        <span class="hint">Uses <code>osascript</code> (macOS) or <code>notify-send</code> (Linux) on the machine running the daemon. Not available in the Docker image: there, use the browser or ntfy.</span>
      </Panel>
      <Panel title="ntfy" hint="Also send them to your phone through an ntfy topic.">
        <label class="f">Server
          <input class="mono" bind:value={form.ntfyServer} placeholder="https://ntfy.example.com" />
          <span class="hint">Off until server and topic are both set. Messages carry agent names and reasons (never PR comments or documents) and go to this server, in plain text: use your own, or treat the topic as a password.</span></label>
        <label class="f">Topic
          <input class="mono" bind:value={form.ntfyTopic} placeholder="xherd-your-secret-name" />
          <span class="hint">Letters, digits, - and _.</span></label>
      </Panel>
    </div>
  {:else if tab === 'plugins' && open}
    <a href={href('/settings', { tab: 'plugins' })}>← All plugins</a>
    <Panel title={open.label || open.name} hint={`${open.kind} · ${open.version}${open.builtin ? ' · built in' : ''} · ${open.enabled ? 'on' : 'off'}`}>
      {#if open.description}<p class="muted">{open.description}</p>{/if}
      {#each open.settings as s (s.key)}
        {#if s.secret}
          <div class="f">{s.label || s.key}<span class="hint">Secret: kept in secrets.json, not editable here yet.</span></div>
        {:else if plugins[open.name]}
          <label class="f">{s.label || s.key}{#if s.required} *{/if}
            <input bind:value={plugins[open.name][s.key]} placeholder={s.default != null ? String(s.default) : ''} /></label>
        {/if}
      {:else}
        <p class="empty">This plugin has no settings.</p>
      {/each}
    </Panel>
    {#if open.kind === 'toolchain'}
      {#key open.name}<Toolchains plugin={open.name} />{/key}
    {/if}
  {:else}
    {#snippet pcard(p: PluginInfo)}
      <a class="pcard" href={href('/settings', { tab: 'plugins', plugin: p.name })}>
        <div class="h">
          <span class="mi {p.kind}" aria-hidden="true">{initials(p)}</span>
          <div class="wrap-any">
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
    {#if broken.length}
      <div class="cards">
        {#each broken as b (b.dir)}
          <article class="pcard err">
            <div class="h"><span class="mi bad" aria-hidden="true">!</span>
              <div class="wrap-any"><h3>{b.name}</h3><div class="hint mono">{b.dir}</div></div></div>
            <div class="b"><p class="err">{b.error}</p></div>
          </article>
        {/each}
      </div>
    {/if}
    <div class="sect"><h2>Enabled</h2></div>
    <div class="cards">
      {#each plugs.filter((p) => p.enabled) as p (p.name)}{@render pcard(p)}{/each}
    </div>
    {#if plugs.some((p) => !p.enabled)}
      <div class="sect"><h2>Available</h2><span class="hint">Installed but switched off. Switch them on in the Welcome page tools step.</span></div>
      <div class="cards">
        {#each plugs.filter((p) => !p.enabled) as p (p.name)}{@render pcard(p)}{/each}
      </div>
    {/if}
  {/if}
  {#if setup.config && ready && ['general', 'gates', 'notifications', 'plugins'].includes(tab) && (tab !== 'plugins' || open)}
    <SaveBar {dirty} busy={saving} onsave={save} ondiscard={() => setup.config && fill(setup.config)} />
  {/if}
</div>
