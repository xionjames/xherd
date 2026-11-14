<script lang="ts">
  // ⌘K / Ctrl+K: filter agents by name or ticket and open the agent page.
  import Icon from './Icon.svelte';
  import { app } from '../lib/store.svelte';
  import { go } from '../lib/router.svelte';

  let q = $state('');
  let open = $state(false);
  let sel = $state(0);
  let input: HTMLInputElement;

  const hits = $derived(
    (app.state?.agents ?? [])
      .filter((a) => q && [a.name, a.ticket?.key ?? '', a.ticket?.url ?? ''].join(' ').toLowerCase().includes(q.toLowerCase()))
      .slice(0, 8),
  );

  function onGlobalKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      input.focus();
      input.select();
    }
  }

  function pick(i: number) {
    const a = hits[i];
    if (!a) return;
    q = '';
    open = false;
    input.blur();
    go(`/agent/${a.name}`);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { sel = Math.min(sel + 1, hits.length - 1); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { sel = Math.max(sel - 1, 0); e.preventDefault(); }
    else if (e.key === 'Enter') pick(sel);
    else if (e.key === 'Escape') { q = ''; input.blur(); }
  }
</script>

<svelte:window onkeydown={onGlobalKey} />

<div class="jwrap">
  <label class="jump">
    <Icon name="search" />
    <input bind:this={input} bind:value={q} placeholder="Jump to an agent or ticket…" aria-label="Jump to an agent or ticket"
      onfocus={() => (open = true)} onblur={() => setTimeout(() => (open = false), 120)}
      oninput={() => (sel = 0)} onkeydown={onKey} role="combobox" aria-expanded={open && hits.length > 0}
      aria-controls="jump-list" />
    <span class="kbd">⌘K</span>
  </label>
  {#if open && hits.length}
    <ul id="jump-list" class="jlist" role="listbox">
      {#each hits as a, i (a.id)}
        <li role="option" aria-selected={i === sel}>
          <button class:on={i === sel} onmousedown={() => pick(i)}>
            <span class="mono">{a.name}</span><span class="muted">{a.status}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .jwrap { position: relative; margin-right: 8px; }
  /* No ⌘K on a phone; the herd strip lists the agents there. */
  @media (max-width: 760px) { .jwrap { display: none; } }
  .jump input { all: unset; flex: 1; min-width: 0; color: var(--bar-ink); font-size: 12.5px; }
  .jump input::placeholder { color: var(--bar-ink-2); }
  .jlist { position: absolute; z-index: 40; top: 38px; left: 0; right: 0; margin: 0; padding: 4px; list-style: none;
    background: var(--pen); border: 1px solid var(--fence-strong); border-radius: 8px; box-shadow: 0 10px 30px rgba(0,0,0,.2); }
  .jlist button { display: flex; justify-content: space-between; gap: 8px; width: 100%; padding: 6px 8px; border: 0;
    border-radius: 5px; background: transparent; color: var(--ink); cursor: pointer; font: inherit; }
  .jlist button.on { background: var(--accent-wash); }
</style>
