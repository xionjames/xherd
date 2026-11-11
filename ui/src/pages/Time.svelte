<script lang="ts">
  // Work log (spec §16.4): agent time and your time per day and ticket, and
  // per-ticket delivery metrics measured from what happened.
  import { get } from '../lib/api';
  import { href } from '../lib/router.svelte';
  import { span } from '../lib/format';
  import { showError } from '../lib/errors.svelte';

  type Day = { day: string; tickets: { ticket: string; agentMs: number; userMs: number }[] };
  type Metric = { ticket: string; agent: string; status: string; cycleMs: number | null; reviewRounds: number; autoApprovals: number; rework: number };
  let data = $state<{ days: Day[]; tickets: Metric[] } | null>(null);
  let days = $state('14');
  $effect(() => {
    const n = days;
    get<{ days: Day[]; tickets: Metric[] }>(`/worklog?days=${n}`).then((r) => (data = r)).catch((e) => showError('Could not load the work log', e));
  });
  const total = (d: Day, k: 'agentMs' | 'userMs') => d.tickets.reduce((n, t) => n + t[k], 0);
</script>

<div class="stack">
  <div class="phead notabs"><div class="row"><h1>Time</h1>
    <span class="grow"></span>
    <select aria-label="Period" bind:value={days}>
      <option value="7">Last 7 days</option><option value="14">Last 14 days</option><option value="30">Last 30 days</option><option value="90">Last 90 days</option>
    </select></div></div>
  {#if !data}
    <p class="muted">Loading…</p>
  {:else}
    <section class="stack">
      <h2>Per day</h2>
      <p class="hint">Agent time is time spent running; your time is time with an agent's page open and in front. Days are UTC.</p>
      {#if !data.days.length}<p class="muted">Nothing recorded in this period.</p>{/if}
      {#each data.days as d (d.day)}
        <div class="card" style="padding:12px 16px">
          <div class="row"><b class="mono">{d.day}</b><span class="grow"></span>
            <span class="hint">agents {span(total(d, 'agentMs'))} · you {span(total(d, 'userMs'))}</span></div>
          <table class="tbl">
            <tbody>
              {#each d.tickets as t (t.ticket)}
                <tr><td class="mono">{t.ticket}</td><td class="num">agent {span(t.agentMs)}</td><td class="num">you {span(t.userMs)}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/each}
    </section>
    <section class="stack">
      <h2>Delivery per ticket</h2>
      <p class="hint">Cycle time runs from creating the agent to the pull request merged or closed. Rework counts auto-approvals you later asked changes on, on the same gate.</p>
      {#if !data.tickets.length}<p class="muted">No agents yet.</p>{:else}
        <table class="tbl card">
          <thead><tr><th>Ticket</th><th>Agent</th><th class="num">Cycle time</th><th class="num">Review rounds</th><th class="num">Auto-approved</th><th class="num">Rework</th></tr></thead>
          <tbody>
            {#each data.tickets as t (t.agent)}
              <tr>
                <td class="mono">{t.ticket}</td>
                <td><a href={href(`/agent/${t.agent}`)}>{t.agent}</a></td>
                <td class="num">{t.cycleMs === null ? 'open' : span(t.cycleMs)}</td>
                <td class="num">{t.reviewRounds}</td>
                <td class="num">{t.autoApprovals}</td>
                <td class="num">{t.rework}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
  {/if}
</div>

<style>
  .tbl { width: 100%; border-collapse: collapse; font-size: 13px; }
  .tbl th, .tbl td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--fence-soft); }
  .tbl .num { text-align: right; font-variant-numeric: tabular-nums; }
</style>
