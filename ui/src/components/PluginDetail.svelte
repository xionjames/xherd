<script lang="ts">
  // One plugin's options in the Welcome wizard: its settings (saved here), its
  // checks with their fixes, and for a toolchain its tools and versions. The
  // same things Settings → Plugins shows, in the place where the plugin is picked.
  import { get, put, post } from '../lib/api';
  import { boundProvider } from '../lib/gitProviders';
  import { loadChecks, loadConfig, setup } from '../lib/setup.svelte';
  import { app } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import Panel from './Panel.svelte';
  import CheckRow from './CheckRow.svelte';
  import Toolchains from './Toolchains.svelte';
  import TermView from './TermView.svelte';
  import type { PluginInfo, GitHostsView } from '../lib/types';

  let { plugin, onclose }: { plugin: PluginInfo; onclose: () => void } = $props();

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
  const dirty = $derived(secretsDirty || fieldsDirty);
  let busy = $state(false);
  const checks = $derived((setup.checks ?? []).filter((c) => c.id.startsWith(`plugin:${plugin.name}:`)));
  const loginFails = $derived(plugin.login && checks.some((c) => c.id.endsWith(':login') && !c.ok));

  let loggingIn = $state(false);
  async function login() {
    try { await post(`/plugins/${plugin.name}/login`); loggingIn = true; } catch (e) { showError(`Could not log in to ${plugin.label || plugin.name}`, e); }
  }
  function loginEnded() { loggingIn = false; loadChecks(); }

  async function save() {
    busy = true;
    try {
      const mine = Object.fromEntries(fields.map((s) => [s.key, values[s.key] ?? '']).filter(([, v]) => v !== ''));
      // The server replaces this plugin's settings as a whole, so an emptied field is cleared.
      // Only when a plain setting changed: it needs a daemon restart, a secret does not.
      if (fieldsDirty) await put('/config', { pluginSettings: { [plugin.name]: mine } });
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

  // A git provider (D24): the hosts of Settings → Git access that use it.
  let gitView = $state<GitHostsView | null>(null);
  $effect(() => {
    gitView = null;
    if (plugin.kind === 'host') get<GitHostsView>('/git/hosts').then((v) => (gitView = v)).catch(() => {});
  });
  const provider = $derived(gitView?.providers.find((p) => p.name === plugin.name));
  const usedBy = $derived(gitView && provider ? gitView.hosts.filter((h) => boundProvider(gitView!.providers, h)?.name === plugin.name) : []);
</script>

<Panel title={`${plugin.label || plugin.name} options`} hint={plugin.description}>
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
  {:else}
    <p class="hint">This plugin has no settings.</p>
  {/if}
  {#if provider}
    <p class="hint">Git provider for {provider.hosts.join(', ')}.
      {#if usedBy.length}Used by {usedBy.map((h) => h.host).join(', ')}.{:else}No host in Git access uses it yet.{/if}
      <a href="#/settings?tab=git">Git access</a></p>
  {/if}
  {#if plugin.kind === 'toolchain'}
    {#key plugin.name}<Toolchains plugin={plugin.name} />{/key}
  {/if}
  {#snippet foot()}
    {#if loginFails}<button class="btn" onclick={login}>Log in</button>{/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={onclose}>Close</button>
    {#if fields.length || secretFields.length}<button class="btn primary" disabled={!dirty || busy} onclick={save}>{busy ? 'Saving…' : 'Save settings'}</button>{/if}
  {/snippet}
</Panel>
{#if checks.length}
  <Panel title="Checks" flush>
    {#each checks as c, i (c.id)}<CheckRow check={c} first={i === 0} />{/each}
  </Panel>
{/if}
{#if loggingIn}
  <TermView path="/ws/setup/plugin-login" title={`Log in to ${plugin.label || plugin.name}`} height="360px" onclose={loginEnded}
    hint="Follow the steps; the check runs again when it ends." />
{/if}
