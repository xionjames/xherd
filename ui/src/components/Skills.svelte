<script lang="ts">
  // Settings → Skills (R25): the library agents draw from. A skill is a folder
  // with a SKILL.md; it is copied into a job's worktree when the job starts.
  import { del, get, post } from '../lib/api';
  import { showError } from '../lib/errors.svelte';
  import { fromFolder, NAME_HINT, type SkillFilePost } from '../lib/skills';
  import type { SkillDetail, SkillInfo } from '../lib/types';
  import Panel from './Panel.svelte';
  import Dialog from './Dialog.svelte';
  import Callout from './Callout.svelte';

  let skills = $state<SkillInfo[]>([]);
  let loaded = $state(false);
  let viewing = $state<SkillDetail | null>(null);
  let removing = $state<SkillInfo | null>(null);

  // adding: from text, or from a folder the browser read
  let adding = $state(false);
  let name = $state('');
  let text = $state('');
  let files = $state<SkillFilePost[] | null>(null);
  let skipped = $state(0);
  let replace = $state(false);
  let busy = $state(false);
  let problem = $state('');

  const kb = (n: number) => (n < 1024 ? `${n} B` : `${Math.max(1, Math.round(n / 1024))} KiB`);
  const template = (n: string) => `---\nname: ${n || 'my-skill'}\ndescription: What it does and when to use it\n---\n\nSteps for the agent…\n`;

  async function load() {
    try {
      skills = (await get<{ skills: SkillInfo[] }>('/skills')).skills;
      loaded = true;
    } catch (e) {
      showError('Could not list skills', e);
    }
  }
  $effect(() => { load(); });

  export function start() {
    adding = true; name = ''; text = template(''); files = null; skipped = 0; replace = false; problem = '';
  }
  async function pickFolder(e: Event) {
    const list = Array.from((e.currentTarget as HTMLInputElement).files ?? []);
    if (!list.length) return;
    const picked = await Promise.all(list.map(async (f) => ({
      path: (f as File & { webkitRelativePath: string }).webkitRelativePath || f.name,
      bytes: new Uint8Array(await f.arrayBuffer()),
    })));
    const r = fromFolder(picked);
    files = r.files;
    skipped = r.skipped;
    name = r.name;
    problem = r.files.some((f) => f.path === 'SKILL.md') ? '' : 'The folder has no SKILL.md at its top.';
  }
  function useText() { files = null; skipped = 0; problem = ''; }

  async function add() {
    busy = true;
    problem = '';
    try {
      const body = files ? { name, files, replace } : { name, text, replace };
      await post<SkillDetail>('/skills', body);
      adding = false;
      await load();
    } catch (e) {
      problem = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  async function view(s: SkillInfo) {
    try {
      viewing = await get<SkillDetail>(`/skills/${encodeURIComponent(s.name)}`);
    } catch (e) {
      showError('Could not open the skill', e);
    }
  }
  async function remove(s: SkillInfo) {
    removing = null;
    try {
      await del(`/skills/${encodeURIComponent(s.name)}`);
      await load();
    } catch (e) {
      showError('Could not delete the skill', e);
    }
  }
  const nameOk = $derived(NAME_HINT.test(name));
</script>

<div class="secs">
  <Panel title="Skills" hint="Reusable instructions and files. Agents list the ones they use; each job gets its own copy in its worktree, never committed." flush>
    {#if loaded && !skills.length}<div class="cb"><p class="empty">No skills yet.</p></div>{/if}
    {#if skills.length}
      <ul class="list">
        {#each skills as s (s.name)}
          <li>
            <div class="row" style="flex-wrap:nowrap">
              <div class="grow wrap-any">
                <b class="mono">{s.name}</b>
                <div class="hint">{s.description}</div>
                <div class="hint">{s.files} file{s.files === 1 ? '' : 's'} · {kb(s.bytes)} · {s.usedBy.length ? `used by ${s.usedBy.join(', ')}` : 'not used by any agent'}</div>
              </div>
              <button class="btn sm" onclick={() => view(s)}>View</button>
              <button class="btn ghost danger sm" disabled={s.usedBy.length > 0} title={s.usedBy.length ? 'An agent lists it: remove it from the agent first' : ''} onclick={() => (removing = s)}>Delete</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </Panel>

  {#if adding}
    <Panel title="New skill">
      <div class="grp">
        <label class="f">From a folder
          <input type="file" webkitdirectory onchange={pickFolder} />
          <span class="hint">A folder with a <span class="mono">SKILL.md</span> at its top. Hidden files are left out. At most 1 MiB and 200 files.</span></label>
        {#if files}
          <p class="hint">{files.length} file{files.length === 1 ? '' : 's'} read{skipped ? `, ${skipped} hidden left out` : ''}. <button class="btn ghost sm" onclick={useText}>Write the text instead</button></p>
        {/if}
        <label class="f">Name <input class="mono" bind:value={name} maxlength="41" placeholder="lint-fix" />
          <span class="hint">Lower-case letters, digits and dashes. The <span class="mono">name:</span> in SKILL.md must be the same.</span></label>
        {#if !files}
          <label class="f">SKILL.md
            <textarea class="mono" rows="10" bind:value={text}></textarea></label>
        {/if}
        <label class="f chk"><input type="checkbox" bind:checked={replace} /> Replace a skill of this name (jobs already started keep their copy)</label>
        {#if problem}<Callout tone="bad">{problem}</Callout>{/if}
      </div>
      {#snippet foot()}
        <button class="btn ghost" onclick={() => (adding = false)}>Cancel</button>
        <button class="btn primary" disabled={busy || !nameOk || (!!files && !!problem)} onclick={add}>{busy ? 'Creating…' : 'Create skill'}</button>
      {/snippet}
    </Panel>
  {/if}
</div>

{#if viewing}
  {@const v = viewing}
  <Dialog title="Skill {v.name}" onclose={() => (viewing = null)}>
    <p class="muted">{v.description}</p>
    <ul class="hint mono">
      {#each v.fileList as f (f.path)}<li>{f.path} · {kb(f.bytes)}</li>{/each}
    </ul>
    <pre class="mono" style="white-space:pre-wrap;max-height:40vh;overflow:auto">{v.text}</pre>
    {#snippet actions()}<button class="btn" onclick={() => (viewing = null)}>Close</button>{/snippet}
  </Dialog>
{/if}

{#if removing}
  {@const r = removing}
  <Dialog title="Delete skill {r.name}?" onclose={() => (removing = null)}>
    <p>It leaves the library. Jobs already started keep their copy.</p>
    {#snippet actions()}
      <button class="btn ghost" onclick={() => (removing = null)}>Keep it</button>
      <button class="btn danger" onclick={() => remove(r)}>Delete skill</button>
    {/snippet}
  </Dialog>
{/if}
