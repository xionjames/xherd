<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import JumpBox from './JumpBox.svelte';
  import ThemeButton from './ThemeButton.svelte';
  import { route, href } from '../lib/router.svelte';
  import { failingRequired } from '../lib/setup.svelte';
  let { blocked = false }: { blocked?: boolean } = $props();
  const failing = $derived(failingRequired().length);
</script>

<header class="bar">
  <a class="logo" href={href('/')} aria-label="xherd home"><Logo /><b>XHERD</b></a>
  <nav class="gnav" aria-label="Main">
    <a class="gt" class:on={route.path === '/' || route.path.startsWith('/agent')}
       aria-current={route.path === '/' ? 'page' : undefined}
       aria-disabled={blocked} href={blocked ? undefined : href('/')} style={blocked ? 'opacity:.45' : ''}>Herd</a>
    {#if !blocked}<a class="gt" class:on={route.path === '/review'} aria-current={route.path === '/review' ? 'page' : undefined}
       href={href('/review')}>To review</a>{/if}
    {#if failing || route.path === '/setup'}<a class="gt" class:on={route.path === '/setup'} aria-current={route.path === '/setup' ? 'page' : undefined}
       href={href('/setup')}>Setup checks{#if failing}<span class="gcount g">{failing}</span>{/if}</a>{/if}
  </nav>
  <div class="grow"></div>
  {#if !blocked}<JumpBox />{/if}
  <ThemeButton />
  <a class="bb" class:on={route.path === '/settings'} href={href('/settings')} aria-label="Settings"
     aria-current={route.path === '/settings' ? 'page' : undefined}><Icon name="settings" /></a>
  {#if !blocked}
    <a class="btn newb" href={href('/new')}><Icon name="plus" />New agent</a>
  {/if}
</header>
