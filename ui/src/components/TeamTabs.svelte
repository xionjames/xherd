<script lang="ts">
  // The tabs of the Team section: who (Agents), what each stage uses (Flows),
  // and what they can do (Skills).
  import Tabs from './Tabs.svelte';
  import { href } from '../lib/router.svelte';
  let { current }: { current: 'agents' | 'flows' | 'skills' } = $props();
  const items = [
    { id: 'agents', label: 'Agents', href: href('/agents') },
    { id: 'flows', label: 'Flows', href: href('/flows') },
    { id: 'skills', label: 'Skills', href: href('/skills') },
  ];
</script>

<Tabs label="Team" {items} {current} />
