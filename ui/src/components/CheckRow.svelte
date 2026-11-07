<script lang="ts">
  import type { Check } from '../lib/types';
  import { fixCheck, loadChecks, setup } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import TermView from './TermView.svelte';
  let { check, first = false }: { check: Check; first?: boolean } = $props();
  const st = $derived(check.ok ? 'ok' : check.required ? 'bad' : 'opt');
  // A session fix runs in a terminal here.
  let session = $state(false);

  async function fix() {
    try {
      await fixCheck(check.id);
      if (check.fix?.kind === 'session') session = true;
    } catch (e) {
      showError(`Could not fix ${check.label}`, e);
    }
  }
  function ended() {
    session = false;
    loadChecks().catch((e) => showError('Could not run the checks', e));
  }
</script>

<div class="ck" class:first style={st === 'bad' ? 'background:var(--halt-wash)' : ''}>
  <span class="ckst {st}" aria-label={check.ok ? 'passes' : check.required ? 'fails' : 'optional, fails'}>
    {check.ok ? '✓' : check.required ? '✕' : '–'}
  </span>
  <div style="min-width:0">
    <div class="row" style="gap:8px">
      <b style="font-weight:500">{check.label}</b>
      {#if !check.required && !check.ok}<span class="pill">optional</span>{/if}
      {#if check.ok}<span class="mono muted">{check.detail}</span>{/if}
    </div>
    {#if !check.ok}
      <p style="color:{check.required ? 'var(--halt)' : 'var(--ink-3)'};word-break:break-word">{check.detail}</p>
      {#if check.fix?.kind === 'command'}<div class="fix"><span style="user-select:all">{check.fix.text}</span></div>{/if}
      {#if check.fix?.kind === 'session'}<p class="hint" style="margin-top:6px">{check.fix.explain}</p>{/if}
    {/if}
  </div>
  {#if !check.ok && check.fix && check.fix.kind !== 'command' && !session}
    <button class="btn primary sm" disabled={setup.running} onclick={fix}>{check.fix.label}</button>
  {:else}<span></span>{/if}
</div>
{#if session}
  <div style="padding:0 18px 16px">
    <TermView path={`/ws/setup/${check.id}`} title={check.label} height="420px" onclose={ended}
      hint="Quit when done; the check runs again." />
  </div>
{/if}
