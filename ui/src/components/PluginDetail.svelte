<script lang="ts">
  // One plugin's options in a dialog (Welcome step 2 and Settings → Plugins):
  // its settings (saved here), its checks with their fixes, for a toolchain its
  // tools and versions, and the plugin's login terminal (the only one).
  import { onMount } from 'svelte';
  import { get, put, post, pluginUpdatePreview, pluginUpdate, pluginRemove } from '../lib/api';
  import { reload } from '../lib/store.svelte';
  import Callout from './Callout.svelte';
  import { boundProvider } from '../lib/gitProviders';
  import { KIND_ONE } from '../lib/plugins';
  import { loadChecks, loadConfig, setup } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import Dialog from './Dialog.svelte';
  import CheckRow from './CheckRow.svelte';
  import Toolchains from './Toolchains.svelte';
  import TermView from './TermView.svelte';
  import ModelInput from './ModelInput.svelte';
  import { validModel } from '../lib/models';
  import type { PluginInfo, GitHostsView, PluginUpdatePreview } from '../lib/types';

  let { plugin, onclose, autologin = false }: { plugin: PluginInfo; onclose: () => void; autologin?: boolean } = $props();
  const label = $derived(plugin.label || plugin.name);

  const saved = (k: string) => String(((setup.config?.config.pluginSettings ?? {}) as Record<string, Record<string, unknown>>)[plugin.name]?.[k] ?? '');
  const fields = $derived(plugin.settings.filter((s) => !s.secret));
  let values = $state<Record<string, string>>({});
  $effect(() => { values = Object.fromEntries(fields.map((s) => [s.key, saved(s.key)])); });
  // Secret settings are write-only: the daemon says which have a value, never what it is.
  const secretFields = $derived(plugin.settings.filter((s) => s.secret));
  let secretSet = $state<Record<string, boolean>>({});
  let secretNew = $state<Record<string, string>>({});
  $effect(() => {
    secretNew = {};
    secretSet = {};
    if (secretFields.length) get<Record<string, boolean>>(`/plugins/${plugin.name}/secrets`).then((v) => (secretSet = v)).catch(() => {});
  });
  async function forget(key: string) {
    try { secretSet = await put<Record<string, boolean>>(`/plugins/${plugin.name}/secrets`, { values: { [key]: null } }); await loadChecks(); }
    catch (e) { showError('Could not remove the secret', e); }
  }
  const secretsDirty = $derived(secretFields.some((s) => (secretNew[s.key] ?? '') !== ''));
  const fieldsDirty = $derived(fields.some((s) => (values[s.key] ?? '') !== saved(s.key)));
  // A runtime that can choose a model: its default model (config `runtimeModels`, live).
  const canModel = $derived(plugin.kind === 'runtime' && !!plugin.chooseModel);
  const savedModels = $derived((setup.config?.config.runtimeModels ?? {}) as Record<string, string>);
  let model = $state('');
  $effect(() => { model = savedModels[plugin.name] ?? ''; });
  const modelDirty = $derived(canModel && model.trim() !== (savedModels[plugin.name] ?? ''));
  const modelOk = $derived(validModel(model));
  const dirty = $derived(secretsDirty || fieldsDirty || modelDirty);
  let busy = $state(false);
  const checks = $derived((setup.checks ?? []).filter((c) => c.id.startsWith(`plugin:${plugin.name}:`)));
  const loginFails = $derived(plugin.enabled && plugin.login && checks.some((c) => c.id.endsWith(':login') && !c.ok));

  let loggingIn = $state(false);
  async function login() {
    try { await post(`/plugins/${plugin.name}/login`); loggingIn = true; } catch (e) { showError(`Could not log in to ${label}`, e); }
  }
  function loginEnded() { loggingIn = false; loadChecks(); }
  // Log in on a card opens this dialog and starts the login here.
  // Closing the dialog during a login drops the terminal before it reports the end,
  // so the checks run again here.
  onMount(() => {
    if (autologin) login();
    return () => { if (loggingIn) loadChecks(); };
  });

  async function save() {
    busy = true;
    try {
      const mine = Object.fromEntries(fields.map((s) => [s.key, values[s.key] ?? '']).filter(([, v]) => v !== ''));
      // The server replaces this plugin's settings as a whole, so an emptied field is cleared.
      // Only when a plain setting changed: it needs a daemon restart, a secret does not.
      if (fieldsDirty) await put('/config', { pluginSettings: { [plugin.name]: mine } });
      // The map is replaced as a whole; an empty model removes the entry.
      if (modelDirty) await put('/config', { runtimeModels: { ...savedModels, [plugin.name]: model.trim() } });
      const fresh = Object.fromEntries(Object.entries(secretNew).filter(([, v]) => v !== ''));
      if (Object.keys(fresh).length) {
        secretSet = await put<Record<string, boolean>>(`/plugins/${plugin.name}/secrets`, { values: fresh });
        secretNew = {};
      }
      await loadConfig();
      await loadChecks();
    } catch (e) {
      showError('Could not save the plugin settings', e);
    } finally {
      busy = false;
    }
  }

  // Update and Remove (R8, R9) of a plugin installed from a git URL or by hand;
  // built-ins have neither. Each takes the dialog's place so only one is open.
  let mode = $state<'options' | 'update' | 'remove'>('options');
  let acting = $state(false);
  let actionError = $state('');
  let updateRef = $state('');
  let upd = $state<PluginUpdatePreview | null>(null);
  let updated = $state('');
  let forgetSettings = $state(false);
  const sourceText = $derived(plugin.source ? `${plugin.source.url}${plugin.source.sha ? `@${plugin.source.sha.slice(0, 7)}` : ''}` : '');
  const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));
  function back() { mode = 'options'; upd = null; updated = ''; actionError = ''; }
  async function lookForUpdate() {
    acting = true;
    actionError = '';
    try { upd = await pluginUpdatePreview(plugin.name, updateRef.trim() || undefined); }
    catch (e) { upd = null; actionError = errText(e); }
    finally { acting = false; }
  }
  async function applyUpdate() {
    if (!upd?.digest) return;
    acting = true;
    actionError = '';
    try {
      const r = await pluginUpdate(plugin.name, upd.digest, updateRef.trim() || undefined);
      updated = `${r.version} at ${r.sha.slice(0, 7)}`;
      await reload();
      await loadConfig().catch(() => {});
    } catch (e) { actionError = errText(e); }
    finally { acting = false; }
  }
  async function removeIt() {
    acting = true;
    actionError = '';
    try {
      await pluginRemove(plugin.name, forgetSettings);
      await reload();
      await loadConfig().catch(() => {});
      onclose();
    } catch (e) { actionError = errText(e); acting = false; }
  }

  // A Git provider (D24): the hosts of Settings → Git access that use it.
  let gitView = $state<GitHostsView | null>(null);
  $effect(() => {
    gitView = null;
    if (plugin.kind === 'host') get<GitHostsView>('/git/hosts').then((v) => (gitView = v)).catch(() => {});
  });
  const provider = $derived(gitView?.providers.find((p) => p.name === plugin.name));
  const usedBy = $derived(gitView && provider ? gitView.hosts.filter((h) => boundProvider(gitView!.providers, h)?.name === plugin.name) : []);
</script>

{#if mode === 'update'}
<Dialog title={`Update ${label}`} onclose={back}>
  {#if updated}
    <Callout tone="ok">Updated <b>{plugin.name}</b> to <span class="mono">{updated}</span>.</Callout>
    <p>Restart needed: the new version loads when xherd restarts. Running agents keep going.</p>
  {:else}
    <p class="hint">Installed: <span class="mono wrap-any">{sourceText}</span>{plugin.source?.ref ? ` · ${plugin.source.ref}` : ''}</p>
    <form class="stack-in" onsubmit={(e) => { e.preventDefault(); lookForUpdate(); }}>
      <label class="f">Branch, tag or commit (optional)
        <input class="mono" bind:value={updateRef} oninput={() => (upd = null)} placeholder={plugin.source?.ref || 'the default branch'} autocomplete="off" spellcheck="false" /></label>
      <div class="row"><button class="btn" type="submit" disabled={acting}>{acting && !upd ? 'Looking…' : 'Check for update'}</button></div>
    </form>
    {#if actionError}<Callout tone="bad"><span class="wrap-any">{actionError}</span></Callout>{/if}
    {#if upd}
      <dl class="kv">
        <dt>Version</dt><dd>{upd.installed.version ?? '?'} → {upd.version}</dd>
        <dt>Commit</dt><dd class="mono">{upd.installed.sha?.slice(0, 7) ?? '?'} → {upd.sha.slice(0, 7)}</dd>
        <dt>Size</dt><dd>{upd.files} files</dd>
      </dl>
      {#if upd.warning}<Callout tone="need"><span class="wrap-any">{upd.warning}</span></Callout>{/if}
      {#if upd.compatible === false}<Callout tone="bad">Needs xherd {upd.minXherdVersion} or newer.</Callout>{/if}
      <span class="lbl">Manifest changes</span>
      {#if upd.manifestDiff}<pre class="diff mono">{upd.manifestDiff}</pre>{:else}<p class="hint">The manifest is unchanged.</p>{/if}
      {#if upd.runs?.length}
        <span class="lbl">It will run</span>
        <ul class="plain">{#each upd.runs as r, i (i)}<li><span class="hint">{r.what}</span> <code class="mono wrap-any">{r.argv.join(' ')}</code></li>{/each}</ul>
      {/if}
    {/if}
  {/if}
  {#snippet actions()}
    <span class="grow"></span>
    <button class="btn" onclick={back}>{updated ? 'Done' : 'Cancel'}</button>
    {#if !updated}<button class="btn primary" disabled={acting || !upd?.digest || upd.compatible === false} onclick={applyUpdate}>{acting && upd ? 'Updating…' : 'Update'}</button>{/if}
  {/snippet}
</Dialog>
{:else if mode === 'remove'}
<Dialog title={`Remove ${label}`} onclose={back}>
  <p>Remove <b>{label}</b> from this xherd? Its folder is deleted; the repository it came from is not touched. A plugin that a running agent uses cannot be removed.</p>
  <label class="f chk"><input type="checkbox" bind:checked={forgetSettings} /> Also forget its saved settings and secrets</label>
  <p class="hint">The plugin stays loaded until xherd restarts.</p>
  {#if actionError}<Callout tone="bad"><span class="wrap-any">{actionError}</span></Callout>{/if}
  {#snippet actions()}
    <span class="grow"></span>
    <button class="btn" onclick={back}>Cancel</button>
    <button class="btn primary danger" disabled={acting} onclick={removeIt}>{acting ? 'Removing…' : 'Remove'}</button>
  {/snippet}
</Dialog>
{:else}
<Dialog title={`${label} options`} {onclose}>
  <p class="muted">{plugin.description}</p>
  <p class="hint">{KIND_ONE[plugin.kind] ?? plugin.kind} · {plugin.version}{plugin.builtin ? ' · built in' : ''}{plugin.enabled ? '' : ' · switched off'}</p>
  {#if plugin.source}<p class="hint wrap-any">Installed from <span class="mono">{sourceText}</span></p>
  {:else if !plugin.builtin}<p class="hint">Installed by hand, not pinned to a repository.</p>{/if}
  {#if loggingIn}
    <TermView path="/ws/setup/plugin-login" title={`Log in to ${label}`} height="360px" onclose={loginEnded}
      hint="Follow the steps; the check runs again when it ends." />
  {/if}
  {#if plugin.settings.length}
    {#each fields as s (s.key)}
      <label class="f">{s.label || s.key}{#if s.required} *{/if}
        <input bind:value={values[s.key]} placeholder={s.default != null ? String(s.default) : ''} /></label>
    {/each}
    {#each secretFields as s (s.key)}
      <label class="f">{s.label || s.key}{#if s.required} *{/if}
        <input type="password" autocomplete="off" bind:value={secretNew[s.key]}
          placeholder={secretSet[s.key] ? 'stored · type to replace' : 'not set'} />
        {#if secretSet[s.key]}<button class="btn ghost danger sm" style="align-self:flex-start" onclick={(e) => { e.preventDefault(); forget(s.key); }}>Forget the stored value</button>{/if}</label>
    {/each}
    {#if secretFields.length}<p class="hint">Secrets are kept in secrets.json on the daemon's side and never shown again.</p>{/if}
  {:else if !canModel}
    <p class="hint">This plugin has no settings.</p>
  {/if}
  {#if canModel}
    <label class="f">Default model
      <ModelInput runtime={plugin.name} bind:value={model} />
      {#if !modelOk}<span class="hint err">Letters, digits and . _ : / @ - [ ] only.</span>
      {:else}<span class="hint">New agents of this runtime start with it; empty = the runtime's own default. A repo profile or the New agent form can choose another, and each agent can change it. Applies at once.</span>{/if}</label>
  {/if}
  {#if provider}
    <p class="hint">Git provider for {provider.hosts.join(', ')}.
      {#if usedBy.length}Used by {usedBy.map((h) => h.host).join(', ')}.{:else}No host in Git access uses it yet.{/if}
      <a href="#/settings?tab=git">Git access</a></p>
  {/if}
  {#if plugin.kind === 'toolchain'}
    {#key plugin.name}<Toolchains plugin={plugin.name} />{/key}
  {/if}
  {#if checks.length}
    <span class="lbl">Checks</span>
    <div class="card">
      {#each checks as c, i (c.id)}<CheckRow check={c} first={i === 0} />{/each}
    </div>
  {:else if !plugin.enabled}
    <p class="hint">Its checks run once it is switched on.</p>
  {/if}
  {#snippet actions()}
    {#if loginFails && !loggingIn}<button class="btn" onclick={login}>Log in</button>{/if}
    {#if plugin.source}<button class="btn" onclick={() => (mode = 'update')}>Update</button>{/if}
    {#if !plugin.builtin}<button class="btn ghost danger" onclick={() => (mode = 'remove')}>Remove</button>{/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={onclose}>Close</button>
    {#if fields.length || secretFields.length || canModel}<button class="btn primary" disabled={!dirty || busy || !modelOk} onclick={save}>{busy ? 'Saving…' : 'Save settings'}</button>{/if}
  {/snippet}
</Dialog>
{/if}

<style>
  .stack-in { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .diff { margin: 0; padding: 8px 10px; max-height: 240px; overflow: auto; white-space: pre; background: var(--well); border: 1px solid var(--fence-soft); border-radius: var(--r-ctl); font-size: 12px; }
</style>
