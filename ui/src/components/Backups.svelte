<script lang="ts">
  // Backups (spec §15, R-1401): the archives in the backup folder and a button
  // to take one now. Restoring is `xherd backup restore` with the daemon stopped.
  import { get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { ago } from '../lib/format';

  type Item = { name: string; size: number; at: number };
  let dir = $state('');
  let items = $state<Item[]>([]);
  let busy = $state(false);
  let notes = $state<string[]>([]);
  let loaded = $state(false);

  const mb = (n: number) => (n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KiB` : `${(n / 1024 / 1024).toFixed(1)} MiB`);

  async function load() {
    try {
      const v = await get<{ dir: string; backups: Item[] }>('/backups');
      dir = v.dir;
      items = v.backups;
      loaded = true;
    } catch (e) {
      showError('Could not list backups', e);
    }
  }
  async function now() {
    busy = true;
    try {
      const r = await post<{ warnings: string[] }>('/backups', { label: 'manual' });
      notes = r.warnings;
      await load();
    } catch (e) {
      showError('The backup failed', e);
    } finally {
      busy = false;
    }
  }
  $effect(() => { load(); });
</script>

<div class="secs">
  <section class="sec">
    <div class="sh"><h2>Backups</h2><span class="hint">Database, settings, plugins and each unfinished agent's notes, commits and uncommitted work. No tokens or logins.</span></div>
    <div class="bd">
      <p class="muted">Folder: <span class="mono">{dir}</span></p>
      <div><button class="btn primary" disabled={busy} onclick={now}>{busy ? 'Backing up…' : 'Back up now'}</button></div>
      {#each notes as n}<p class="hint">{n}</p>{/each}
      {#if loaded && !items.length}<p class="muted">No backups yet.</p>{/if}
      {#each items as b (b.name)}
        <div class="hint"><span class="mono">{b.name}</span> · {mb(b.size)} · {ago(b.at)}</div>
      {/each}
    </div>
  </section>
  <section class="sec">
    <div class="sh"><h2>Restore or move to another computer</h2></div>
    <div class="bd">
      <p class="muted">Stop the daemon, then on the new computer run
        <span class="mono">xherd backup restore &lt;archive&gt; --workspace &lt;folder&gt;</span>.
        Agents come back paused; resume the ones you want. Git tokens and logins are entered again. <span class="mono">xherd backup verify</span> checks an archive first.</p>
    </div>
  </section>
</div>
