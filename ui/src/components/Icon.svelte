<script lang="ts">
  import { ICONS } from '../lib/icons';
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg class="ic" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true"><path d={ICONS[name]} /></svg>
