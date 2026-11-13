<script lang="ts">
  // A model name for a runtime: free text with the runtime's suggestions.
  import { app } from '../lib/store.svelte';
  import { modelText, runtimeModels } from '../lib/models';

  let { runtime, value = $bindable(''), placeholder = "runtime's default", disabled = false, label = 'Model', onchange }: {
    runtime: string; value?: string; placeholder?: string; disabled?: boolean; label?: string;
    onchange?: (v: string) => void;
  } = $props();

  const id = $props.id();
  const suggestions = $derived(runtimeModels(app.state?.plugins, runtime) ?? []);
</script>

<input class="mono" aria-label={label} list={`models-${id}`} bind:value {placeholder} {disabled}
  autocomplete="off" spellcheck="false" onchange={() => onchange?.(value)} />
<datalist id={`models-${id}`}>{#each suggestions as s (s.id)}<option value={s.id}>{modelText(s)}</option>{/each}</datalist>
