<script lang="ts">
  // Who agents commit as (written to the managed gitconfig; per-owner rules
  // become includeIf blocks). Separate from the git hosts: git refuses to
  // commit without it, whatever the remote. Used by Settings and Welcome.
  import { get, put } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import Panel from './Panel.svelte';
  import SaveBar from './SaveBar.svelte';
  import Callout from './Callout.svelte';
  import GitconfigNote from './GitconfigNote.svelte';
  import type { ConfigView } from '../lib/types';

  // `bar` off in Welcome, whose single button saves through `save()`; `unsaved`
  // and `ready` (name and email filled in) are read back by the page.
  let { heading = true, bar = true, unsaved = $bindable(false), ready = $bindable(false) }: {
    heading?: boolean; bar?: boolean; unsaved?: boolean; ready?: boolean;
  } = $props();

  // Who agents commit as: git refuses to commit without it, and the container
  // has no identity of its own.
  let idName = $state('');
  let idEmail = $state('');
  let idRules = $state<{ scope: string; name: string; email: string }[]>([]);
  let idSaved = $state('');
  const idSnapshot = () => `${idName}\n${idEmail}\n${JSON.stringify(idRules)}`;
  let idBusy = $state(false);
  const idDirty = $derived(idSnapshot() !== idSaved);
  $effect(() => { unsaved = idDirty; ready = !!idName.trim() && !!idEmail.trim(); });
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
  export async function save() { if (idDirty) await saveIdentity(); return !idDirty; }
  async function saveIdentity() {
    idBusy = true;
    try {
      await put('/config', { gitUserName: idName, gitUserEmail: idEmail, gitIdentities: idRules });
      idRules = idRules.map((r) => ({ ...r, scope: r.scope.trim().replace(/\/+$/, ''), name: r.name.trim(), email: r.email.trim() }));
      idSaved = idSnapshot();
    } catch (e) { showError('Could not save the commit identity', e); }
    finally { idBusy = false; }
  }
</script>

  <GitconfigNote />
  {#if heading}<div class="sect"><h2>Commit identity</h2></div>{/if}
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

{#if bar}<SaveBar dirty={idDirty} busy={idBusy} onsave={saveIdentity} ondiscard={loadIdentity}
  saveLabel="Save identity" saved="Saved; applies right away" />{/if}

<style>
  .idrule { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 1fr) auto; gap: 10px; align-items: end; }
  @media (max-width: 760px) {
    .idrule { grid-template-columns: minmax(0, 1fr); padding-bottom: 12px; border-bottom: 1px solid var(--fence-soft); }
    .idrule .btn { justify-self: start; }
  }
</style>
