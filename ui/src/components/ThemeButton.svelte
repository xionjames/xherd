<script lang="ts">
  import Icon from './Icon.svelte';
  import { theme, cycle } from '../lib/theme.svelte';
  const ICON = { system: 'auto', light: 'sun', dark: 'moon' };
  const LABEL = { system: 'Theme: system (click for light)', light: 'Theme: light (click for dark)', dark: 'Theme: dark (click for system)' };
</script>

<button class="bb" aria-label={LABEL[theme.value]} title={LABEL[theme.value]} onclick={cycle}>
  <Icon name={ICON[theme.value]} />
</button>
