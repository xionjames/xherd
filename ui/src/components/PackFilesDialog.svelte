<script lang="ts">
  // Decide what happens to the pack files xherd found edited or obsolete (R6).
  // Nothing is rewritten or removed until Apply.
  import type { PackChoice, PackChoiceValue, PackFile } from '../lib/types';
  import { loadPackReview, resolvePackFiles, setup } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import Dialog from './Dialog.svelte';
  import Callout from './Callout.svelte';
  import FilePath from './FilePath.svelte';
  import Seg from './Seg.svelte';
  let { onclose }: { onclose: () => void } = $props();

  let files = $state<PackFile[] | null>(null);
  // Keyed by runtime and path; the defaults keep the user's files.
  let picks = $state<Record<string, PackChoiceValue>>({});
  const key = (f: PackFile) => `${f.pack}\u0000${f.runtime}\u0000${f.path}`;

  $effect(() => {
    loadPackReview().then(
      (list) => {
        files = list;
        picks = Object.fromEntries(list.map((f) => [key(f), f.state === 'edited' ? 'keep-mine' : 'remove']));
      },
      (e) => {
        showError('Could not load the pack files', e);
        onclose();
      },
    );
  });

  const usesNew = $derived((files ?? []).some((f) => picks[key(f)] === 'use-new'));

  async function apply() {
    if (!files) return;
    const choices: PackChoice[] = files.map((f) => ({ pack: f.pack, runtime: f.runtime, path: f.path, choice: picks[key(f)] }));
    try {
      await resolvePackFiles(choices);
      onclose();
    } catch (e) {
      showError('Could not apply the choices', e);
    }
  }
</script>

<Dialog title="Pack files" {onclose}>
  {#if files === null}
    <p class="muted">Loading…</p>
  {:else if files.length === 0}
    <p class="muted">Nothing waits for a decision.</p>
  {:else}
    <p class="hint">xherd updates its own command files at every start. These ones need your decision.</p>
    <ul class="stack" style="list-style:none;padding:0;margin:var(--pad) 0 0">
      {#each files as f (key(f))}
        <li class="row" style="gap:12px;justify-content:space-between;align-items:center">
          <div style="min-width:0;flex:1">
            <FilePath path={f.path} />
            <div class="row" style="gap:6px">
              <span class="muted">{f.runtime}</span>
              <span class="pill">{f.state === 'edited' ? 'changed by you' : 'no longer in the pack'}</span>
              {#if f.state === 'obsolete' && f.edited}<span class="pill">edited</span>{/if}
            </div>
          </div>
          {#if f.state === 'edited'}
            <Seg label={`Choice for ${f.path}`} value={picks[key(f)]} onchange={(v) => (picks[key(f)] = v)}
              options={[{ value: 'keep-mine', label: 'Keep mine' }, { value: 'use-new', label: "Use xherd's version" }]} />
          {:else}
            <Seg label={`Choice for ${f.path}`} value={picks[key(f)]} onchange={(v) => (picks[key(f)] = v)}
              options={[{ value: 'remove', label: 'Remove' }, { value: 'keep', label: 'Keep' }]} />
          {/if}
        </li>
      {/each}
    </ul>
    {#if usesNew}
      <div style="margin-top:var(--pad)">
        <Callout tone="need">Using xherd's version replaces your changes to that file.</Callout>
      </div>
    {/if}
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!files?.length || setup.running} onclick={apply}>Apply</button>
  {/snippet}
</Dialog>
