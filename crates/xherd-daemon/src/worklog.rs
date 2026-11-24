//! Work log and delivery metrics (spec §16.4): agent time (time in `running`,
//! flushed each minute), the user's time on an agent page (heartbeats from the
//! UI) and, per ticket, cycle time, review rounds and rework.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use xherd_core::Status;
use xherd_core::usage::day;
use xherd_store::Event;

use crate::{ActionError, Daemon};

/// Longest span one heartbeat or flush may add: a stuck client or a daemon
/// that was suspended must not book hours.
const MAX_SPAN_MS: u64 = 120_000;

/// Prefix of the work log rows kept per saved agent (the key column holds a
/// ticket otherwise).
const AGENT_KEY: &str = "agent:";

/// Auto-approvals (`approval` events with `auto: true`) later followed by a
/// change request on the same gate (`change-request` events), per agent.
/// Both lists are oldest first.
pub fn rework(approvals: &[Event], requests: &[Event]) -> BTreeMap<String, (u32, u32)> {
    let mut out: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for a in approvals {
        let p = a.payload.as_ref();
        if p.and_then(|p| p["auto"].as_bool()) != Some(true) {
            continue;
        }
        let gate = p.and_then(|p| p["gate"].as_str()).unwrap_or("");
        let redone = requests.iter().any(|r| {
            r.job_id == a.job_id
                && r.at >= a.at
                && r.payload.as_ref().and_then(|p| p["gate"].as_str()) == Some(gate)
        });
        let e = out.entry(a.job_id.clone()).or_default();
        e.0 += 1;
        e.1 += u32::from(redone);
    }
    out
}

impl Daemon {
    /// Once a minute: book the time since the last flush to every running
    /// agent's ticket.
    pub async fn flush_worklog(&self) {
        let now = self.clock.now();
        let last = self
            .worklog_flushed
            .swap(now, std::sync::atomic::Ordering::Relaxed);
        if last == 0 {
            return;
        }
        let ms = ((now - last).max(0) as u64).min(MAX_SPAN_MS);
        let Ok(agents) = self.store.list_jobs().await else {
            return;
        };
        for a in agents.iter().filter(|a| a.status == Status::Running) {
            if let Err(e) = self
                .store
                .add_worklog(&day(now), a.work_item(), "agent", ms)
                .await
            {
                tracing::warn!(error = %e, "work log not written");
            }
            self.book_agent(&day(now), a, "agent", ms).await;
        }
    }

    /// The same time under the saved agent the job came from (R24), as a row
    /// keyed `agent:<id>`; the per-ticket listing leaves those out.
    async fn book_agent(&self, day: &str, job: &xherd_core::Job, kind: &str, ms: u64) {
        if let Some(a) = job.agent()
            && let Err(e) = self
                .store
                .add_worklog(day, &format!("{AGENT_KEY}{}", a.id), kind, ms)
                .await
        {
            tracing::warn!(error = %e, "work log not written");
        }
    }

    /// The agent page was open and visible for `ms` (a heartbeat from the UI).
    pub async fn add_user_time(&self, who: &str, ms: u64) -> Result<(), ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let (today, ms) = (day(self.clock.now()), ms.min(MAX_SPAN_MS));
        self.book_agent(&today, &a, "user", ms).await;
        self.store
            .add_worklog(&today, a.work_item(), "user", ms)
            .await
            .map_err(|e| ActionError {
                code: "store",
                message: e.to_string(),
            })
    }

    /// `GET /worklog`: time per day and ticket, and delivery metrics per
    /// ticket (cycle time, review rounds, rework). With `by_agent` the days
    /// list saved agents (by name) instead of tickets.
    pub async fn worklog(&self, days: u32, by_agent: bool) -> Result<Value, ActionError> {
        let io = |e: xherd_store::StoreError| ActionError {
            code: "store",
            message: e.to_string(),
        };
        let now = self.clock.now();
        let since = day(now - i64::from(days.clamp(1, 366)) * 86_400_000);
        let rows = self.store.worklog(&since).await.map_err(io)?;
        let mut by_day: BTreeMap<String, BTreeMap<String, (u64, u64)>> = BTreeMap::new();
        let mut labels: BTreeMap<String, String> = BTreeMap::new();
        let names: BTreeMap<String, String> = if by_agent {
            let defs = self.store.list_agent_defs().await.map_err(io)?;
            for a in &defs {
                let l = xherd_core::agent_def::label(&a.name, a.source.as_deref());
                labels.insert(a.name.clone(), l.to_string());
            }
            defs.into_iter().map(|a| (a.id, a.name)).collect()
        } else {
            BTreeMap::new()
        };
        for (d, t, kind, ms) in rows {
            let t = match (t.strip_prefix(AGENT_KEY), by_agent) {
                (Some(id), true) => names.get(id).cloned().unwrap_or_else(|| id.to_string()),
                (None, false) => t,
                _ => continue,
            };
            let e = by_day.entry(d).or_default().entry(t).or_default();
            if kind == "user" {
                e.1 += ms;
            } else {
                e.0 += ms;
            }
        }
        let days_json: Vec<Value> = by_day
            .into_iter()
            .rev()
            .map(|(d, t)| {
                let tickets: Vec<Value> = t
                    .into_iter()
                    .map(|(k, (a, u))| {
                        let mut row = json!({"ticket": k, "agentMs": a, "userMs": u});
                        if let Some(l) = labels.get(&k) {
                            row["label"] = json!(l);
                        }
                        row
                    })
                    .collect();
                json!({"day": d, "tickets": tickets})
            })
            .collect();
        let approvals = self.store.events_of_kind("approval").await.map_err(io)?;
        let requests = self
            .store
            .events_of_kind("change-request")
            .await
            .map_err(io)?;
        let rw = rework(&approvals, &requests);
        let agents = self.store.list_jobs().await.map_err(io)?;
        let tickets: Vec<Value> = agents
            .iter()
            .map(|a| {
                let (auto, redone) = rw.get(&a.id).copied().unwrap_or_default();
                let finished = a.status == Status::Done;
                json!({
                    "ticket": a.work_item(),
                    "job": a.name,
                    "status": a.status,
                    "cycleMs": finished.then(|| (a.status_since - a.created_at).max(0)),
                    "reviewRounds": a.review_rounds,
                    "autoApprovals": auto,
                    "rework": redone,
                })
            })
            .collect();
        Ok(json!({"days": days_json, "tickets": tickets}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(agent: &str, kind: &str, at: i64, gate: &str, auto: bool) -> Event {
        Event {
            id: format!("{kind}{at}"),
            job_id: agent.into(),
            kind: kind.into(),
            payload: Some(json!({"gate": gate, "auto": auto})),
            at,
        }
    }

    #[test]
    fn rework_counts_auto_approvals_changed_on_the_same_gate() {
        let approvals = [
            ev("a", "approval", 10, "design", true),
            ev("a", "approval", 20, "tasks", true),
            ev("a", "approval", 30, "spec", false),
        ];
        let requests = [
            ev("a", "change-request", 15, "design", false),
            ev("a", "change-request", 5, "tasks", false),
        ];
        // `tasks` was changed before it was auto-approved: not rework.
        assert_eq!(rework(&approvals, &requests)["a"], (2, 1));
    }
}
