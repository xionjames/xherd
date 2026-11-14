<script lang="ts">
  // The files of the diff shown, with their counts (R15): a filter box (R16,
  // the list only) and a click that takes you to the file's section. On a
  // phone the list folds behind a "Files (N)" toggle (R17).
  import { filterFiles, type FileDiff } from '../lib/diff';
  import FilePath from './FilePath.svelte';

  let { files, label = 'Files', onpick }: { files: FileDiff[]; label?: string; onpick: (path: string) => void } = $props();
  let q = $state('');
  let open = $state(false);
  const shown = $derived(filterFiles(files, q));
</script>

<section class="card cb fnav" class:open>
  <button class="btn sm tog" aria-expanded={open} onclick={() => (open = !open)}>
    {open ? '▾' : '▸'} {label} ({files.length})
  </button>
  <span class="lbl head">{label} · {files.length}</span>
  <div class="fold">
    {#if files.length > 1}
      <input type="search" placeholder="Filter files" aria-label="Filter files" bind:value={q} />
    {/if}
    <ul>
      {#each shown as f (f.path)}
        <li>
          <button class="pick" onclick={() => onpick(f.path)} title={f.path}>
            <span class="p"><FilePath path={f.path} /></span>
            <span class="num adds">+{f.adds}</span><span class="num dels">−{f.dels}</span>
          </button>
        </li>
      {:else}
        <li class="empty">No file matches.</li>
      {/each}
    </ul>
  </div>
</section>

<style>
  .fnav { display: flex; flex-direction: column; gap: 8px; }
  .tog { display: none; align-self: flex-start; }
  ul { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; }
  .pick {
    display: flex; align-items: center; gap: 8px; width: 100%; min-width: 0;
    padding: 4px 6px; border: 0; border-radius: 6px; background: none; color: var(--ink);
    font: inherit; font-size: 12.5px; text-align: left; cursor: pointer;
  }
  .pick:hover { background: var(--accent-wash); }
  .pick:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .p { flex: 1 1 auto; min-width: 0; }
  .adds { color: var(--live); font-weight: 600; }
  .dels { color: var(--halt); font-weight: 600; }
  .empty { font-size: 12.5px; padding: 4px 6px; }
  @media (max-width: 759px) {
    .tog { display: inline-flex; }
    .head { display: none; }
    .fnav:not(.open) .fold { display: none; }
  }
</style>
