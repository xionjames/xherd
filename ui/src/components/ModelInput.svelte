<script lang="ts">
  // A model for a runtime: a select of the runtime's suggestions with
  // "Other…" for any name, or a text box when the runtime lists none.
  import { modelList, modelText, type ModelList } from '../lib/models';

  // `inline`: sized by its content, next to other controls (the agent page
  // header), instead of filling a form field.
  let { runtime, value = $bindable(''), placeholder = "Default (the runtime's own)", disabled = false, label = 'Model', inline = false, onchange }: {
    runtime: string; value?: string; placeholder?: string; disabled?: boolean; label?: string; inline?: boolean;
    onchange?: (v: string) => void;
  } = $props();

  const OTHER = '\u0000other';
  let list = $state<ModelList | null>(null);
  let custom = $state(false);
  $effect(() => {
    const rt = runtime;
    list = null;
    modelList(rt).then((l) => { if (rt === runtime) list = l; }).catch(() => { if (rt === runtime) list = { choose: true, models: [], error: 'not listed' }; });
  });
  const models = $derived(list?.models ?? []);
  // A saved name the list does not have still shows as an option.
  const extra = $derived(value && !custom && !models.some((m) => m.id === value) ? value : '');

  function pick(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === OTHER) { custom = true; return; }
    custom = false;
    value = v;
    onchange?.(v);
  }
</script>

{#if list && models.length === 0}
  <input class:mono={!inline} aria-label={label} bind:value {placeholder} {disabled} autocomplete="off" spellcheck="false"
    title={list.error ? `No model list: ${list.error}` : undefined} onchange={() => onchange?.(value)} />
{:else}
  <span class="mpick" class:inline>
    <select class:mono={!inline} aria-label={label} value={custom ? OTHER : value} {disabled} onchange={pick}>
      <option value="">{placeholder}</option>
      {#if extra}<option value={extra}>{extra}</option>{/if}
      {#each models as m (m.id)}<option value={m.id} title={m.id}>{modelText(m)}</option>{/each}
      {#if !list}<option disabled>Loading models…</option>{/if}
      <option value={OTHER}>Other…</option>
    </select>
    {#if custom}
      <!-- svelte-ignore a11y_autofocus -->
      <input class="mono" aria-label={`${label} name`} bind:value placeholder="model name" {disabled} autofocus
        autocomplete="off" spellcheck="false" onchange={() => onchange?.(value)} />
    {/if}
  </span>
{/if}

<style>
  .mpick { display: flex; gap: 6px; width: 100%; min-width: 0; }
  .mpick select, .mpick input { flex: 1 1 0; min-width: 0; }
  .mpick.inline { display: inline-flex; width: auto; }
  .mpick.inline select, .mpick.inline input { flex: none; }
</style>
