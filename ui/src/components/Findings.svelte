<script lang="ts">
  // Findings of `review.md` with a colour per severity (R-114). The colour is
  // never the only signal: each severity also has a glyph and its name.
  import type { Job, Finding, Severity } from '../lib/types';
  import { get, post } from '../lib/api';
  import { upsertJob } from '../lib/store.svelte';
  import { showError } from '../lib/errors.svelte';
  import Seg from './Seg.svelte';

  let { agent }: { agent: Job } = $props();
  // Effects key on the id: the agent object is replaced on every update.
  const id = $derived(agent.id);
  // Reload when the document or the tasks that fix findings change.
  const key = $derived(
    (agent.gate?.kind === 'docs' ? agent.gate.files.map((f) => f.sha256).join() : '') +
      (agent.tasks ?? []).map((t) => t.status).join(),
  );

  const GLYPH: Record<Severity, string> = { blocker: '■', major: '▲', minor: '●' };
  const ORDER: Severity[] = ['blocker', 'major', 'minor'];

  let findings = $state<Finding[]>([]);
  let error = $state<string | null>(null);

  $effect(() => {
    void key;
    get<{ findings: Finding[]; error?: string }>(`/jobs/${id}/findings`).then(
      (r) => { findings = r.findings; error = r.error ?? null; },
      (e) => { findings = []; error = e.message; },
    );
  });

  const sorted = $derived([...findings].sort((a, b) => ORDER.indexOf(a.severity) - ORDER.indexOf(b.severity)));
  const count = (s: Severity) => findings.filter((f) => f.severity === s && !f.resolved).length;

  // Fix or skip per open finding, sent as one change request (like the PR
  // comments' decisions). Skipped and undecided findings stay as they are.
  type Choice = 'fix' | 'skip';
  let choices = $state<Record<string, Choice>>({});
  let sending = $state(false);
  const decidable = (f: Finding) => !f.resolved && !f.fixed?.length;
  const toFix = $derived(findings.filter((f) => decidable(f) && choices[f.id] === 'fix'));
  const toSkip = $derived(findings.filter((f) => decidable(f) && choices[f.id] === 'skip'));
  const canSend = $derived(agent.gate?.kind === 'docs' && agent.status !== 'blocked');
  async function sendFixes() {
    const line = (f: Finding) => `- ${f.id} (${f.severity}) ${f.title}${f.location ? ` at ${f.location}` : ''}`;
    const comment = [
      'Fix these review findings. For each, add a task with `source: review` to tasks.md, run `xherd tasks sync`, implement them one by one, then run `xherd stage enter review` again and update review.md:',
      ...toFix.map(line),
      ...(toSkip.length ? ['', 'Do not fix these; leave them in review.md as left, with the reason:', ...toSkip.map(line)] : []),
    ].join('\n');
    sending = true;
    try {
      const r = await post<{ job: Job }>(`/jobs/${id}/request-changes`, { comment });
      upsertJob(r.job);
      choices = {};
    } catch (e) {
      showError('Could not send the findings', e);
    } finally {
      sending = false;
    }
  }

  function stateText(f: Finding): string {
    if (f.resolved) return `fixed by ${f.fixed?.join(', ')} ✓`;
    if (f.fixed?.length) return `waiting for ${f.fixed.join(', ')}`;
    if (f.left) return `left: ${f.left}`;
    return 'not fixed';
  }
</script>

{#if error}
  <div class="card cb"><p class="err">review.md: {error}</p></div>
{:else if findings.length}
  <section class="card" aria-label="Review findings">
    <div class="ch">
      <h2>Findings</h2>
      <span class="row">
        {#each ORDER as s (s)}
          {#if count(s)}<span class="pill sev-{s}">{GLYPH[s]} {count(s)} {s}</span>{/if}
        {/each}
        {#if !findings.some((f) => !f.resolved)}<span class="pill live">✓ all fixed</span>{/if}
        {#if toFix.length}
          <button class="btn gate sm" disabled={sending || !canSend} onclick={sendFixes}
            title={canSend ? 'Sends them to the agent; you stay at this gate' : 'The agent is not waiting at a gate'}>Fix {toFix.length} finding{toFix.length === 1 ? '' : 's'}</button>
        {/if}
      </span>
    </div>
    <div class="tscroll">
      <table class="tbl">
        <thead><tr><th>Severity</th><th>Finding</th><th class="hide-sm">Location</th><th>State</th><th>Action</th></tr></thead>
        <tbody>
          {#each sorted as f (f.id)}
            <tr class="sev-{f.severity}" class:done={f.resolved}>
              <td><span class="pill sev-{f.severity}">{GLYPH[f.severity]} {f.severity}</span></td>
              <td><span class="mono muted">{f.id}</span> {f.title}
                {#if f.location}<div class="hint mono sm-only">{f.location}</div>{/if}</td>
              <td class="mono muted hide-sm">{f.location ?? ''}</td>
              <td class="muted state">{stateText(f)}</td>
              <td>
                {#if decidable(f) && canSend}
                  <Seg label={`Action for ${f.id}`} value={choices[f.id] ?? ('' as Choice)} onchange={(c) => (choices[f.id] = c)}
                    options={[{ value: 'fix', label: '✎ fix', tone: 'acc' }, { value: 'skip', label: '○ skip' }]} />
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>
{/if}

<style>
  tr td:first-child { box-shadow: inset 3px 0 0 var(--sev-minor); }
  tr.sev-blocker td:first-child { box-shadow: inset 3px 0 0 var(--sev-blocker); }
  tr.sev-major td:first-child { box-shadow: inset 3px 0 0 var(--sev-major); }
  tr.done td { opacity: .6; }
  .state { font-size: 12px; }
  .sm-only { display: none; }
  @media (max-width: 760px) { .sm-only { display: block; } }
</style>
