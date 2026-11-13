<script lang="ts">
  // A model name for a runtime: free text with the runtime's suggestions.
  import { modelList, modelText, type ModelList } from '../lib/models';

  let { runtime, value = $bindable(''), placeholder = "runtime's default", disabled = false, label = 'Model', onchange }: {
    runtime: string; value?: string; placeholder?: string; disabled?: boolean; label?: string;
    onchange?: (v: string) => void;
  } = $props();

  const id = $props.id();
  let list = $state<ModelList | null>(null);
  $effect(() => {
    const rt = runtime;
    list = null;
    modelList(rt).then((l) => { if (rt === runtime) list = l; }).catch(() => {});
  });
</script>

<input class="mono" aria-label={label} list={`models-${id}`} bind:value {placeholder} {disabled}
  autocomplete="off" spellcheck="false" title={list?.error ? `No suggestions: ${list.error}` : undefined}
  onchange={() => onchange?.(value)} />
<datalist id={`models-${id}`}>{#each list?.models ?? [] as s (s.id)}<option value={s.id}>{modelText(s)}</option>{/each}</datalist>
