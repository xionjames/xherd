<script lang="ts">
  import Icon from './Icon.svelte';
  import { theme, isDark, toggle, followSystem } from '../lib/theme.svelte';
  // The icon is what a click gives you: a sun while dark, a moon while light.
  const label = $derived(
    `Switch to ${isDark() ? 'light' : 'dark'} theme` +
      (theme.value === 'system' ? ' (now following your system; Shift+click keeps it)' : ' (Shift+click: follow your system)'),
  );
</script>

<button class="bb" aria-label={label} title={label} onclick={(e) => (e.shiftKey ? followSystem() : toggle())}>
  <Icon name={isDark() ? 'sun' : 'moon'} />
</button>
