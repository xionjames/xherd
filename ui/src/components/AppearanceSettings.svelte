<script lang="ts">
  // How xherd looks: Theme (per browser, applied at once) and Job view (the
  // daemon's `ui.jobView`, saved when a card is clicked). Shared by welcome
  // step 1 and Settings → General (#202 R14).
  import Seg from './Seg.svelte';
  import { app } from '../lib/store.svelte';
  import { put } from '../lib/api';
  import { setup } from '../lib/setup.svelte';
  import { showError } from '../lib/errors.svelte';
  import { theme, apply, type Theme } from '../lib/theme.svelte';
  import type { ConfigView, JobView } from '../lib/types';

  const THEMES: { value: Theme; label: string }[] = [
    { value: 'system', label: 'System' },
    { value: 'light', label: 'Light' },
    { value: 'dark', label: 'Dark' },
  ];
  const VIEWS: { value: JobView; label: string; text: string }[] = [
    { value: 'chat', label: 'Chat', text: 'Messages, tool calls and a box to write in. Questions and permission requests show up as cards.' },
    { value: 'terminal', label: 'Terminal', text: 'The agent\'s own terminal, as it runs. Every job can still switch between the two.' },
  ];

  // Only a runtime with a chat view makes the choice meaningful.
  const hasChat = $derived((app.state?.plugins ?? []).some((p) => p.kind === 'runtime' && p.enabled && p.chat));
  const saved = $derived(
    (setup.config?.config.ui ?? app.state?.config.ui) as { jobView?: unknown } | undefined,
  );
  const view = $derived<JobView>(saved?.jobView === 'chat' ? 'chat' : 'terminal');

  async function choose(v: JobView) {
    if (v === view) return;
    try {
      setup.config = await put<ConfigView>('/config', { ui: { jobView: v } });
    } catch (e) {
      showError('Could not save', e);
    }
  }
</script>

<div class="appearance">
  <div class="f">Theme
    <Seg label="Theme" options={THEMES} value={theme.value} onchange={apply} />
    <span class="hint">Follows your system by default. Kept in this browser only.</span>
  </div>
  {#if hasChat}
    <div class="f">Job view
      <div class="vcards" role="group" aria-label="Job view">
        {#each VIEWS as v (v.value)}
          <button type="button" class="vcard" aria-pressed={view === v.value} onclick={() => choose(v.value)}>
            <span class="vprev" class:term={v.value === 'terminal'} aria-hidden="true">
              {#if v.value === 'chat'}
                <i class="m a"></i><i class="m b"></i><i class="m c"></i>
              {:else}
                <i class="t"></i><i class="t s"></i><i class="t"></i>
              {/if}
            </span>
            <b>{v.label}</b>
            <span class="hint">{v.text}</span>
          </button>
        {/each}
      </div>
      <span class="hint">What a job opens on. Saved for every browser; a job remembers its own switch here.</span>
    </div>
  {/if}
</div>

<style>
  .appearance { display: flex; flex-direction: column; gap: 16px; }
  .vcards { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 220px), 1fr)); gap: 10px; }
  .vcard { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; text-align: left; padding: 10px; background: var(--pen); border: 1px solid var(--fence); border-radius: var(--r-card); color: var(--ink); cursor: pointer; font: inherit; }
  .vcard:hover { border-color: var(--fence-strong); }
  .vcard[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-wash); box-shadow: 0 0 0 1px var(--accent); }
  .vprev { display: flex; flex-direction: column; gap: 4px; width: 100%; padding: 8px; background: var(--pen-2); border-radius: 6px; }
  .vprev i { display: block; height: 6px; border-radius: 3px; background: var(--fence-strong); }
  .vprev .m.a { width: 70%; }
  .vprev .m.b { width: 55%; align-self: flex-end; background: var(--accent); }
  .vprev .m.c { width: 80%; }
  .vprev.term i { height: 4px; border-radius: 1px; width: 85%; }
  .vprev.term i.s { width: 45%; }
</style>
