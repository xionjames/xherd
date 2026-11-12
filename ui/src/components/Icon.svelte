<script lang="ts">
  import { ICONS } from '../lib/icons';
  let { name, size = 16, solid = false }: { name: string; size?: number; solid?: boolean } = $props();
</script>

<svg class="ic" class:solid viewBox="0 0 24 24" width={size} height={size} aria-hidden="true"><path d={ICONS[name]} /></svg>
