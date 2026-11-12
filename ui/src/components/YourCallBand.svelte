<script lang="ts">
  // The full-width band shown when the agent waits on the human: what to
  // decide, why it is held, and the decision controls.
  import Icon from './Icon.svelte';
  import type { Agent, FlowInfo, GitFinding } from '../lib/types';
  import { get, post } from '../lib/api';
  import { app, upsertAgent } from '../lib/store.svelte';
  import { reads } from '../lib/reads.svelte';
  import { showError } from '../lib/errors.svelte';
  import { ago } from '../lib/format';
  import { stageLabel } from '../lib/flow';
  import { href } from '../lib/router.svelte';

  let { agent, flow, tabId }: { agent: Agent; flow?: FlowInfo; tabId?: string } = $props();
  let text = $state('');
  let busy = $state(false);
  // Open questions are answered in the Documents panel, on their items (R-105a).
  const openQuestions = $derived(agent.gate?.kind === 'docs' ? (agent.gate.questions?.length ?? 0) + (agent.gate.asked?.length ?? 0) : 0);
  // R-688: what the git doctor says about the host a git-access block is about.
  const gitBlocked = $derived(agent.status === 'blocked' && agent.blockReason === 'git access');
  const agentId = $derived(agent.id);
  let doctor = $state<{ host: string; first?: GitFinding; checking: boolean; error?: string } | null>(null);
  async function checkGit() {
    doctor = { host: '', checking: true };
    try {
      const r = await get<{ host: string; findings: GitFinding[] }>(`/agents/${agentId}/git-access`);
      doctor = { host: r.host, first: r.findings.find((f) => !f.ok), checking: false };
    } catch (e) {
      doctor = { host: '', checking: false, error: e instanceof Error ? e.message : String(e) };
    }
  }
  $effect(() => { if (gitBlocked && agentId) checkGit(); else doctor = null; });
  // The band stays one line high; long text opens on demand.
  let open = $state(false);

  const g = $derived(agent.gate);
  // Commit gate: the message xherd commits with, editable before approving.
  let msg = $state('');
  let msgFor = '';
  const proposed = $derived(g?.kind === 'commit' ? (g.message ?? g.subject) : '');
  $effect(() => {
    const key = g?.kind === 'commit' ? `${g.id}|${proposed}` : '';
    if (key !== msgFor) { msgFor = key; msg = proposed; }
  });
  const fresh = $derived(app.state?.config.freshSessions !== false);
  const nextStage = $derived.by(() => {
    if (!flow || !g || g.kind !== 'docs') return undefined;
    const i = flow.stages.findIndex((s) => s.id === g.stage);
    return i >= 0 ? flow.stages[i + 1] : undefined;
  });
  const answering = $derived(!g || g.kind === 'question');
  const approvable = $derived(g?.kind === 'docs' || g?.kind === 'commit');
  // The user approves what they read: the controls show on the tab that
  // holds it; elsewhere the band offers a button that goes there.
  const reviewTab = $derived(g?.kind === 'docs' && g.role !== 'ship' ? 'spec' : g?.kind === 'docs' || g?.kind === 'commit' ? 'changes' : undefined);
  const reviewing = $derived(!approvable || tabId === reviewTab);
  const openLabel = $derived(
    g?.kind === 'docs' && g.role !== 'ship' ? `Open ${g.files[0]?.path.split('/').pop() ?? 'the document'}` : 'Review the changes',
  );
  const reviewHref = $derived(
    href(`/agent/${agent.name}`, g?.kind === 'docs' && g.role !== 'ship' && g.files[0] ? { s: 'spec', doc: g.files[0].path } : { s: reviewTab ?? 'changes' }),
  );

  const title = $derived.by(() => {
    if (!g) return agent.status === 'blocked' ? (agent.blockReason ? `the agent is blocked: ${agent.blockReason}` : 'the agent is blocked') : 'the agent asks you';
    switch (g.kind) {
      case 'docs': return `${g.files.map((f) => f.path).join(', ') || g.stage} ${g.files.length > 1 ? 'are' : 'is'} ready for review`;
      case 'commit': return `commit ${g.id} is ready`;
      case 'question': return 'the agent asks you';
      case 'pr-triage': return `${g.count} new PR review comments`;
      case 'pr-replies': return `${g.count} PR replies to approve`;
    }
  });
  const line = $derived.by(() => {
    const wait = `Waiting ${ago(agent.statusSince)}.`;
    if (!g) return wait;
    if (g.kind === 'docs') {
      const how = fresh ? ' in a fresh session' : '';
      if (g.role === 'ship') return `Approve to push the branch and open the draft PR${how}. ${wait}`;
      return nextStage ? `Approve to start the ${stageLabel(nextStage).toLowerCase()} step${how}. ${wait}` : `${wait}`;
    }
    if (g.kind === 'commit') return `${g.subject} · ${g.files.length} files. Approve to let the agent commit them. ${wait}`;
    return wait;
  });
  const held = $derived(g && 'needsHuman' in g ? g.needsHuman : undefined);
  const question = $derived(g?.kind === 'question' ? g.text : agent.lastMessage);
  const approveLabel = $derived(
    g?.kind === 'commit' ? `Approve ${g.id}` : g?.kind === 'docs' ? (g.role === 'ship' ? 'Approve and ship' : `Approve ${g.stage}`) : 'Approve',
  );

  async function run(action: string, body: Record<string, unknown>, label: string): Promise<boolean> {
    busy = true;
    try {
      const r = await post<{ agent: Agent }>(`/agents/${agent.id}/${action}`, body);
      upsertAgent(r.agent);
      text = '';
      return true;
    } catch (e) {
      showError(label, e);
      return false;
    } finally {
      busy = false;
    }
  }

  function approve() {
    const expect: Record<string, string> = {};
    if (g?.kind === 'docs') {
      for (const f of g.files) {
        const sha = reads[agent.id]?.[f.path];
        if (sha) expect[f.path] = sha;
      }
    }
    const body: Record<string, unknown> = { expect };
    if (g?.kind === 'commit' && msg.trim() && msg.trim() !== proposed.trim()) body.message = msg.trim();
    run('approve', body, 'Could not approve');
  }
</script>

<section class="gband" aria-label="Your call">
  <div class="gtop">
    <div class="ghand" aria-hidden="true"><Icon name="handSolid" size={28} solid /></div>
    <div class="gtitle" class:open class:more={true}>
      <h2>Your call: {title}</h2>
      <p class="muted">{line}</p>
      {#if held}<p class="gwhy">Held: {held}</p>{/if}
      {#if gitBlocked && doctor}
        <p class="gwhy">
          {#if doctor.checking}Checking git access…
          {:else if doctor.error}Could not run the git doctor: {doctor.error}
          {:else if doctor.first}{doctor.host}: {doctor.first.label} — {doctor.first.detail}{#if doctor.first.fix?.kind === 'command'} <span class="mono">{doctor.first.fix.text}</span>{:else if doctor.first.fix} ({doctor.first.fix.label}, in Settings){/if}
          {:else}The doctor finds nothing wrong with {doctor.host} now; message the agent to carry on.{/if}
          {#if !doctor.checking}<button class="btn ghost sm" onclick={checkGit}>Check again</button>{/if}
        </p>
      {/if}
      {#if answering && question}<p class="gwhy">“{question}”</p>{/if}
      {#if agent.memoryProposal}
        <p class="gwhy">The agent proposes an update of AGENTS.md (shared memory):
          {#if tabId === 'memory'}accept or dismiss it below; it never blocks the PR.
          {:else}<a href={href(`/agent/${agent.name}`, { s: 'memory' })}>review it</a>; it never blocks the PR.{/if}</p>
      {/if}
      {#if openQuestions}
        <p class="gwhy">{openQuestions} open question{openQuestions === 1 ? '' : 's'}:
          {#if tabId === 'spec'}answer them in the document, then send them with your comments.
          {:else}<a href={href(`/agent/${agent.name}`, { s: 'spec', ...(g?.kind === 'docs' && g.files[0] ? { doc: g.files[0].path } : {}), q: '1' })}>answer them in the document</a>.{/if}
        </p>
      {/if}
      <button class="gtoggle" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Show less' : 'Show more'}</button>
    </div>
    <div class="gact">
      {#if agent.status === 'blocked'}
        {#if agent.blockReason === 'git access'}
          <a class="btn gate lg" href={href('/settings', { tab: 'git' })}>Fix git access</a>
        {/if}
        <a class="btn gate lg" href={href(`/agent/${agent.name}`, { s: 'terminal' })}><Icon name="term" />Open the terminal</a>
      {/if}
      {#if approvable && !reviewing}
        <a class="btn gate lg" href={reviewHref}><Icon name={reviewTab === 'spec' ? 'doc' : 'diff'} />{openLabel}</a>
      {:else if approvable}
        <textarea bind:value={text} placeholder="Or tell the agent what to change…" aria-label="What to change"></textarea>
        <button class="btn lg" disabled={busy || !text.trim()}
          onclick={() => run('request-changes', { comment: text.trim() }, 'Could not request changes')}>Request changes</button>
        <button class="btn gate lg" disabled={busy} onclick={approve}><Icon name="check" />{approveLabel}</button>
      {:else if answering}
        <textarea bind:value={text} placeholder="Your answer" aria-label="Your answer"></textarea>
        <button class="btn gate lg" disabled={busy || !text.trim()}
          onclick={() => run('send', { text: text.trim() }, 'Could not send')}>Send answer</button>
      {:else}
        <span class="muted">PR follow-up is not in the UI yet.</span>
      {/if}
    </div>
  </div>
  {#if g?.kind === 'commit' && reviewing}
    <div class="gmore">
      <label class="gbox" style="flex:1 1 100%">
        <span class="lbl">Commit message · xherd commits {g.files.length} {g.files.length === 1 ? 'file' : 'files'} with it when you approve</span>
        <textarea class="mono" rows="4" bind:value={msg} aria-label="Commit message"></textarea>
        {#if msg.trim() !== proposed.trim()}<span class="hint">Edited: your text replaces the agent's.</span>{/if}
      </label>
    </div>
  {/if}
</section>
