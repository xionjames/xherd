<script lang="ts" generics="T extends string">
  // Segmented control: pick one of a few views or values inside a panel.
  // For pages and sections use Tabs.
  type Option = { value: T; label: string; disabled?: boolean; title?: string; tone?: string };
  let { options, value, onchange, label }: { options: Option[]; value: T; onchange: (v: T) => void; label: string } = $props();
</script>

<div class="seg" role="group" aria-label={label}>
  {#each options as o (o.value)}
    <button type="button" class={o.tone} aria-pressed={o.value === value} disabled={o.disabled} title={o.title}
      onclick={() => onchange(o.value)}>{o.label}</button>
  {/each}
</div>
