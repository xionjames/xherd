<script lang="ts">
  // The full-width band shown when the agent waits on the human: what to
  // decide, why it is held, and the decision controls.
  import Icon from './Icon.svelte';
  import type { Agent, FlowInfo } from '../lib/types';
  import { post } from '../lib/api';
  import { app, upsertAgent } from '../lib/store.svelte';
  import { reads } from '../lib/reads.svelte';
  import { showError } from '../lib/errors.svelte';
  import { ago } from '../lib/format';
  import { stageLabel } from '../lib/flow';
  import { href } from '../lib/router.svelte';

  let { agent, flow, tabId }: { agent: Agent; flow?: FlowInfo; tabId?: string } = $props();
  let text = $state('');
  let busy = $state(false);
  // Open questions: one answer per question, sent as a change request.
  let answers = $state<string[]>([]);
  const questions = $derived(agent.gate?.kind === 'docs' ? (agent.gate.questions ?? []) : []);
  const answered = $derived(answers.some((a, i) => a?.trim() && i < questions.length));
  function sendAnswers() {
    const lines = questions.flatMap((q, i) => (answers[i]?.trim() ? [`Q: ${q}\nA: ${answers[i].trim()}`] : []));
    run('request-changes', { comment: `Answers to your open questions:\n\n${lines.join('\n\n')}` }, 'Could not send the answers')
      .then((ok) => { if (ok) answers = []; });
  }
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
    if (!g) return agent.status === 'blocked' ? 'the agent is blocked' : 'the agent asks you';
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
    <div class="ghand" aria-hidden="true"><Icon name="hand" size={20} /></div>
    <div class="gtitle" class:open class:more={true}>
      <h2>Your call — {title}</h2>
      <p class="muted">{line}</p>
      {#if held}<p class="gwhy">Held: {held}</p>{/if}
      {#if answering && question}<p class="gwhy">“{question}”</p>{/if}
      <button class="gtoggle" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Show less' : 'Show more'}</button>
    </div>
    <div class="gact">
      {#if agent.status === 'blocked'}
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
        <textarea class="mono" rows="4" bind:value={msg} aria-label="Commit message"
          style="font-size:13px;line-height:1.45;resize:vertical;background:var(--pen)"></textarea>
        {#if msg.trim() !== proposed.trim()}<span class="hint">Edited: your text replaces the agent's.</span>{/if}
      </label>
    </div>
  {/if}
  {#if g?.kind === 'docs' && g.questions?.length}
    <div class="gmore">
      <div class="gbox" style="flex:1 1 100%">
        <span class="lbl">Open questions · {g.questions.length}</span>
        {#if reviewing}
          {#each g.questions as q, i (i)}
            <label class="gbox" style="flex:none">
              <span>{q}</span>
              <textarea rows="2" bind:value={answers[i]} aria-label={`Answer: ${q}`} placeholder="Your answer"
                style="background:var(--pen);resize:vertical"></textarea>
            </label>
          {/each}
          <div>
            <button class="btn" disabled={busy || !answered} onclick={sendAnswers}>Send answers</button>
            <span class="hint">The agent revises the document with them; unanswered questions stay open.</span>
          </div>
        {:else}
          <ul>{#each g.questions as q, i (i)}<li>{q}</li>{/each}</ul>
          <span class="hint">Open the document to answer them.</span>
        {/if}
      </div>
    </div>
  {/if}
</section>
