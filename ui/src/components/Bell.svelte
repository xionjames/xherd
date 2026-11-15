<script lang="ts">
  import Icon from './Icon.svelte';
  import { ago } from '../lib/format';
  import { agentByName } from '../lib/store.svelte';
  import { notices, unread, open as openNotice, markAllRead, clearAll, enableBrowser, disableBrowser, browserBlock, browserEnv, BLOCK_TEXT } from '../lib/notices.svelte';
  let shown = $state(false);
  // Why browser notifications cannot be on (R5), looked at again each time the list opens.
  const why = $derived(shown ? (notices.why ?? browserBlock(browserEnv())) : null);
  const count = $derived(unread());
  const toggle = () => { shown = !shown; if (!shown) markAllRead(); };
  const label = $derived(count ? `Notifications, ${count} new` : 'Notifications');
  const go = (n: (typeof notices.list)[number]) => { openNotice(n); shown = false; };
  // A press anywhere outside the bell and its list closes the list.
  let root = $state<HTMLElement>();
  const outside = (e: PointerEvent) => { if (shown && root && !root.contains(e.target as Node)) toggle(); };
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && shown) toggle(); }} onpointerdown={outside} />

<div class="bell" bind:this={root}>
  <button class="bb" class:on={shown} aria-label={label} title={label} aria-expanded={shown} onclick={toggle}>
    <Icon name="bell" />{#if count}<span class="badge">{count}</span>{/if}
  </button>
  {#if shown}
    <div class="pop" role="region" aria-label="Notifications">
      {#if !notices.list.length}<p class="empty" style="padding:12px">Nothing yet. New ones appear here while xherd is open.</p>{/if}
      <ul>
        {#each notices.list as n (n.key)}
          <li class:unread={!n.read}>
            <button class="item" onclick={() => go(n)} disabled={!agentByName(n.agentId) && !n.agentId.startsWith('todo:')}>
              <span>{n.text}</span><span class="muted num">{ago(n.at)}</span>
            </button>
          </li>
        {/each}
      </ul>
      <div class="foot">
        {#if why}<p class="muted why">{BLOCK_TEXT[why]}</p>{/if}
        {#if notices.browser && !why}
          <button class="btn sm" onclick={disableBrowser}>Turn off browser notifications</button>
        {:else}
          <button class="btn sm" onclick={enableBrowser} disabled={!!why}>Turn on browser notifications</button>
        {/if}
        <span class="grow"></span>
        {#if notices.list.length}<button class="btn ghost sm" onclick={clearAll}>Clear</button>{/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .bell { position: relative; }
  /* Placement comes from the global .pop (a sheet at the bottom on a phone). */
  .pop { width: min(380px, 92vw); max-height: 70vh; overflow: auto; padding: 0; }
  @media (max-width: 560px) { .pop { width: auto; } }
  ul { list-style: none; margin: 0; padding: 4px; }
  .item { display: flex; justify-content: space-between; gap: 10px; width: 100%; text-align: left; padding: 8px 10px; border: 0;
    border-radius: 8px; background: transparent; color: inherit; font: inherit; font-size: 13px; cursor: pointer; }
  .item:hover:not(:disabled) { background: var(--accent-wash); }
  .item:disabled { cursor: default; }
  .unread .item { font-weight: 600; }
  .foot { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px 10px; border-top: 1px solid var(--fence-soft); }
  .why { flex-basis: 100%; margin: 0; font-size: 12px; }
</style>
