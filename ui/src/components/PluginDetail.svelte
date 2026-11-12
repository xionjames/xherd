<script lang="ts">
  // One plugin's options in the Welcome wizard: its settings (saved here), its
  // checks with their fixes, and for a toolchain its tools and versions. The
  // same things Settings → Plugins shows, in the place where the plugin is picked.
  import { put, post } from '../lib/api';
  import { loadChecks, loadConfig, setup } from '../lib/setup.svelte';
  import { app } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import Panel from './Panel.svelte';
  import CheckRow from './CheckRow.svelte';
  import Toolchains from './Toolchains.svelte';
  import TermView from './TermView.svelte';
  import type { PluginInfo } from '../lib/types';

  let { plugin, onclose }: { plugin: PluginInfo; onclose: () => void } = $props();

  const saved = (k: string) => String(((setup.config?.config.pluginSettings ?? {}) as Record<string, Record<string, unknown>>)[plugin.name]?.[k] ?? '');
  const fields = $derived(plugin.settings.filter((s) => !s.secret));
  let values = $state<Record<string, string>>({});
  $effect(() => { values = Object.fromEntries(fields.map((s) => [s.key, saved(s.key)])); });
  const dirty = $derived(fields.some((s) => (values[s.key] ?? '') !== saved(s.key)));
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
      await put('/config', { pluginSettings: { [plugin.name]: mine } });
      await loadConfig();
      await loadChecks();
    } catch (e) {
      showError('Could not save the plugin settings', e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel title={`${plugin.label || plugin.name} options`} hint={plugin.description}>
  {#if fields.length || plugin.settings.length}
    {#each fields as s (s.key)}
      <label class="f">{s.label || s.key}{#if s.required} *{/if}
        <input bind:value={values[s.key]} placeholder={s.default != null ? String(s.default) : ''} /></label>
    {/each}
    {#if plugin.settings.some((s) => s.secret)}
      <p class="hint">A secret setting is kept in secrets.json and is not edited here yet.</p>
    {/if}
  {:else}
    <p class="hint">This plugin has no settings.</p>
  {/if}
  {#if plugin.kind === 'toolchain'}
    {#if app.state?.sandbox}
      {#key plugin.name}<Toolchains plugin={plugin.name} />{/key}
    {:else}
      <p class="hint">xherd runs natively, so it does not install languages on this computer: it uses what it finds. Install the version manager and the versions yourself; the checks below say whether it is found.</p>
    {/if}
  {/if}
  {#snippet foot()}
    {#if loginFails}<button class="btn" onclick={login}>Log in</button>{/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={onclose}>Close</button>
    {#if fields.length}<button class="btn primary" disabled={!dirty || busy} onclick={save}>{busy ? 'Saving…' : 'Save settings'}</button>{/if}
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
