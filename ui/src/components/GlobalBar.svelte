<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import JumpBox from './JumpBox.svelte';
  import ThemeButton from './ThemeButton.svelte';
  import Bell from './Bell.svelte';
  import { route, href } from '../lib/router.svelte';
  import { failingRequired } from '../lib/setup.svelte';
  import { app } from '../lib/store.svelte';
  import { watchAttention, attentionCount } from '../lib/attention.svelte';
  let { blocked = false }: { blocked?: boolean } = $props();
  const failing = $derived(failingRequired().length);
  const attn = $derived(attentionCount());
  $effect(() => { if (!blocked) watchAttention(); });
  const onAttention = $derived(route.path === '/attention' || route.path === '/review');
</script>

<header class="bar">
  <a class="logo" href={href('/')} aria-label="xherd home"><Logo /><b>XHERD</b>{#if app.state?.buildVersion}<span class="ver mono" title="xherd version">{app.state.buildVersion}</span>{/if}</a>
  <nav class="gnav" aria-label="Main">
    <a class="gt home" class:on={route.path === '/' || route.path.startsWith('/agent')}
       aria-current={route.path === '/' ? 'page' : undefined}
       aria-disabled={blocked} href={blocked ? undefined : href('/')}>Herd</a>
    {#if !blocked}<a class="gt" class:on={route.path === '/board'} aria-current={route.path === '/board' ? 'page' : undefined}
       href={href('/board')}>Board</a>{/if}
    {#if !blocked}<a class="gt" class:on={route.path === '/time'} aria-current={route.path === '/time' ? 'page' : undefined}
       href={href('/time')}>Time</a>{/if}
    {#if !blocked}<a class="gt" class:on={route.path === '/todos'} aria-current={route.path === '/todos' ? 'page' : undefined}
       href={href('/todos')}>TODOs</a>{/if}
    {#if !blocked}<a class="gt attn" class:on={onAttention} aria-current={onAttention ? 'page' : undefined}
       href={href('/attention')} title={attn ? `${attn} need your attention` : 'Nothing needs you'}>{#if attn}<span aria-hidden="true">✋</span> {/if}<span class="long">Needs attention</span><span class="short">Attention</span>{#if attn}<span class="gcount g">{attn}</span>{/if}</a>{/if}
    {#if failing || route.path === '/setup'}<a class="gt" class:on={route.path === '/setup'} aria-current={route.path === '/setup' ? 'page' : undefined}
       href={href('/setup')}>Setup checks{#if failing}<span class="gcount g">{failing}</span>{/if}</a>{/if}
  </nav>
  <div class="grow"></div>
  {#if !blocked}<JumpBox />{/if}
  {#if !blocked}<Bell />{/if}
  <ThemeButton />
  <a class="bb" class:on={route.path === '/settings'} href={href('/settings')} aria-label="Settings"
     aria-current={route.path === '/settings' ? 'page' : undefined}><Icon name="settings" /></a>
  {#if !blocked}
    <a class="btn primary newb" href={href('/new')} aria-label="New agent"><Icon name="plus" /><span class="nl">New agent</span></a>
  {/if}
</header>
