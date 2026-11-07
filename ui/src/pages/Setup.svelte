<script lang="ts">
  // Setup checks: what must work before agents can run.
  import CheckRow from '../components/CheckRow.svelte';
  import Icon from '../components/Icon.svelte';
  import { setup, loadChecks, failingRequired } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import { href } from '../lib/router.svelte';

  const checks = $derived(setup.checks ?? []);
  const failing = $derived(failingRequired());
  const optionalFailing = $derived(checks.filter((c) => !c.required && !c.ok).length);
  const GROUPS: [string, string, string][] = [
    ['system', 'This computer', 'needed by every agent'],
    ['runtime', 'Coding agent', 'how agents start'],
    ['plugins', 'Plugins', 'their own checks'],
    ['security', 'Security', 'limits you should know about'],
  ];
  async function rerun() {
    try { await loadChecks(); } catch (e) { showError('Could not run the checks', e); }
  }
</script>

{#if failing.length}
  <section class="gband" aria-label="Setup blocked">
    <div class="gtop">
      <div class="ghand" style="background:var(--halt)" aria-hidden="true"><Icon name="x" size={18} /></div>
      <div class="gtitle">
        <h2>{failing.length} required {failing.length === 1 ? 'check fails' : 'checks fail'} — fix them to start agents</h2>
        <p class="muted">Everything except Setup checks and Settings is locked until they pass.</p>
      </div>
      <div class="gact"><button class="btn lg" disabled={setup.running} onclick={rerun}>Re-run all</button></div>
    </div>
  </section>
{:else}
  <div class="phead notabs">
    <div class="row"><h1>Setup checks</h1><span class="pill live">✓ all required checks pass</span><span class="grow"></span>
      <button class="btn" disabled={setup.running} onclick={rerun}>Re-run all</button></div>
  </div>
{/if}

<div class="stack">
  {#if !setup.checks}
    <p class="muted">Running checks…</p>
  {:else}
    <div class="g3">
      <div class="tile"><span class="muted">Passing</span><b style="color:var(--live)">{checks.filter((c) => c.ok).length}</b></div>
      <div class="tile"><span class="muted">Required, failing</span><b style="color:var(--halt)">{failing.length}</b></div>
      <div class="tile"><span class="muted">Optional, failing</span><b>{optionalFailing}</b></div>
    </div>
    {#each GROUPS as [id, title, hint] (id)}
      {@const rows = checks.filter((c) => c.group === id)}
      {#if rows.length}
        <section class="card">
          <div class="ch"><h2>{title}</h2><span class="hint">{hint}</span>
            {#if id === 'plugins'}<span class="grow"></span><a href={href('/settings', { tab: 'plugins' })}>Manage plugins</a>{/if}</div>
          {#each rows as c, i (c.id)}<CheckRow check={c} first={i === 0} />{/each}
        </section>
      {/if}
    {/each}
  {/if}
</div>
