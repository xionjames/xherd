//! Status reports from hooks and `xherd report`, and how they change an agent
//! (spec §5.4, R-008, R-009, R-015, R-016, R-018).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::{Agent, Millis};
use crate::status::{Status, Trigger};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportEvent {
    // Runtime hooks (`xherd hook <event>`).
    SessionStart,
    UserPromptSubmit,
    PostToolUse,
    Notification,
    Stop,
    // `xherd report --state …` / `--release`.
    Working,
    Idle,
    Blocked,
    Release,
}

impl ReportEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportEvent::SessionStart => "session-start",
            ReportEvent::UserPromptSubmit => "user-prompt-submit",
            ReportEvent::PostToolUse => "post-tool-use",
            ReportEvent::Notification => "notification",
            ReportEvent::Stop => "stop",
            ReportEvent::Working => "working",
            ReportEvent::Idle => "idle",
            ReportEvent::Blocked => "blocked",
            ReportEvent::Release => "release",
        }
    }

    pub fn parse(s: &str) -> Option<ReportEvent> {
        use ReportEvent as E;
        [
            E::SessionStart,
            E::UserPromptSubmit,
            E::PostToolUse,
            E::Notification,
            E::Stop,
            E::Working,
            E::Idle,
            E::Blocked,
            E::Release,
        ]
        .into_iter()
        .find(|e| e.as_str() == s)
    }

    /// Frequent events that are not written to the event log (R-015).
    pub fn is_frequent(self) -> bool {
        matches!(self, ReportEvent::PostToolUse | ReportEvent::Working)
    }
}

/// Wire format of `POST /api/v1/hooks` and of `hook-queue.jsonl` lines (xap.md §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Client-generated id; replays are de-duplicated by it.
    pub id: String,
    pub agent_id: String,
    pub event: ReportEvent,
    #[serde(default)]
    pub payload: Value,
    pub seq: u64,
    pub at: Millis,
}

/// Who produced a report. Daemon reports (screen rules) have their own sequence
/// and never make a real report stale (R-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Agent,
    Daemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Tell the user (browser, ntfy, …).
    Notify(String),
    /// A prompt-submitted or working report acknowledges a pending delivery (R-022).
    DeliveryAck,
    /// The user typed a prompt that is not a slash command (R-015).
    UserPrompted,
    /// Post-stop work: token usage, auto-approval, PR lookup, parking (R-015).
    AfterStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Sequence number not greater than the last one: logged, no change (R-009).
    Stale,
    /// The agent has no session xherd considers live: no revival (R-008).
    Dormant,
    Applied {
        from: Status,
        to: Status,
        effects: Vec<Effect>,
    },
}

fn str_field<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| payload.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
}

/// True when the agent's last message asks the user something.
pub fn asks_question(message: &str) -> bool {
    message.trim_end().ends_with('?')
}

/// Apply one report to the agent. Pure: the caller persists and runs effects.
pub fn apply_report(agent: &mut Agent, r: &Report, origin: Origin, now: Millis) -> Outcome {
    match origin {
        Origin::Agent => {
            if r.seq <= agent.last_seq {
                return Outcome::Stale;
            }
            agent.last_seq = r.seq;
        }
        Origin::Daemon => {
            if r.seq <= agent.last_daemon_seq {
                return Outcome::Stale;
            }
            agent.last_daemon_seq = r.seq;
            // Screen rules only report dialogs appearing and disappearing.
            if !matches!(r.event, ReportEvent::Blocked | ReportEvent::Working) {
                return Outcome::Stale;
            }
        }
    }

    if r.event == ReportEvent::Release {
        agent.session = None;
        agent.resume_cmd = None;
    }
    if !agent.status.is_live() {
        return Outcome::Dormant;
    }

    // R-016: session, activity time. Token usage is computed after stop (AfterStop).
    if origin == Origin::Agent {
        if let Some(s) = str_field(&r.payload, &["session_id", "session", "sessionId"]) {
            agent.session = Some(s.to_string());
        }
        if let Some(c) = str_field(&r.payload, &["resumeCmd", "resume_cmd"]) {
            agent.resume_cmd = Some(c.to_string());
        }
    }
    agent.last_activity = now;

    let from = agent.status;
    let mut effects = Vec::new();
    use ReportEvent as E;
    match r.event {
        E::SessionStart | E::PostToolUse | E::Working | E::UserPromptSubmit => {
            // A screen rule's "dialog gone" only clears a block it caused itself.
            if origin == Origin::Daemon && !agent.screen_blocked {
                return Outcome::Stale;
            }
            agent.fire(Trigger::Working, now);
            if r.event == E::UserPromptSubmit {
                effects.push(Effect::DeliveryAck);
                if let Some(p) = str_field(&r.payload, &["prompt"])
                    && !p.trim_start().starts_with('/')
                {
                    effects.push(Effect::UserPrompted);
                }
            } else if r.event == E::Working {
                effects.push(Effect::DeliveryAck);
            }
        }
        // A runtime notification after the agent stopped is an idle reminder
        // ("waiting for your input"), not a block: permission prompts only
        // come mid-turn. Stopped at a gate, the agent stays approvable.
        E::Notification if matches!(agent.status, Status::WaitingInput | Status::Idle) => {}
        E::Notification | E::Blocked => {
            if origin == Origin::Agent || agent.status != Status::Blocked {
                let was_blocked = agent.status == Status::Blocked;
                agent.fire(Trigger::Blocked, now);
                if !was_blocked {
                    agent.screen_blocked = origin == Origin::Daemon;
                    let msg = str_field(&r.payload, &["message"]).unwrap_or("needs attention");
                    effects.push(Effect::Notify(format!("{} is blocked: {msg}", agent.name)));
                } else if origin == Origin::Agent {
                    agent.screen_blocked = false;
                }
            }
        }
        E::Stop | E::Idle => {
            if let Some(m) = str_field(&r.payload, &["message", "last_assistant_message"]) {
                agent.last_message = Some(m.to_string());
            }
            let question = agent.last_message.as_deref().is_some_and(asks_question);
            let trigger = if agent.gate.is_some() || question {
                Trigger::StoppedAtGate
            } else {
                Trigger::StoppedIdle
            };
            agent.fire(trigger, now);
            agent.generation += 1;
            effects.push(Effect::AfterStop);
        }
        E::Release => {}
    }
    Outcome::Applied {
        from,
        to: agent.status,
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn running() -> Agent {
        let mut a = Agent::new("SC-1", "SC-1", 0);
        a.fire(Trigger::SlotFree, 0);
        a
    }

    fn rep(event: ReportEvent, seq: u64, payload: Value) -> Report {
        Report {
            id: format!("r{seq}"),
            agent_id: "x".into(),
            event,
            payload,
            seq,
            at: 0,
        }
    }

    #[test]
    fn wire_format() {
        let r: Report = serde_json::from_value(json!({
            "id": "01J", "agentId": "a1", "event": "user-prompt-submit",
            "payload": {"prompt": "hi"}, "seq": 3, "at": 5
        }))
        .unwrap();
        assert_eq!(r.event, ReportEvent::UserPromptSubmit);
        assert_eq!(r.seq, 3);
    }

    #[test]
    fn stale_reports_do_not_change_state() {
        let mut a = running();
        apply_report(
            &mut a,
            &rep(ReportEvent::Stop, 5, Value::Null),
            Origin::Agent,
            1,
        );
        assert_eq!(a.status, Status::Idle);
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::Working, 5, Value::Null),
            Origin::Agent,
            2,
        );
        assert_eq!(out, Outcome::Stale);
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::Working, 4, Value::Null),
            Origin::Agent,
            2,
        );
        assert_eq!(out, Outcome::Stale);
        assert_eq!(a.status, Status::Idle);
    }

    #[test]
    fn daemon_reports_never_make_real_reports_stale() {
        let mut a = running();
        apply_report(
            &mut a,
            &rep(ReportEvent::Blocked, 100, Value::Null),
            Origin::Daemon,
            1,
        );
        assert_eq!(a.status, Status::Blocked);
        assert!(a.screen_blocked);
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::Working, 1, Value::Null),
            Origin::Agent,
            2,
        );
        assert!(matches!(
            out,
            Outcome::Applied {
                to: Status::Running,
                ..
            }
        ));
    }

    #[test]
    fn idle_reminder_after_a_stop_does_not_block() {
        let mut a = running();
        a.gate = Some(crate::domain::Gate::Question { text: "?".into() });
        apply_report(
            &mut a,
            &rep(ReportEvent::Stop, 1, Value::Null),
            Origin::Agent,
            1,
        );
        assert_eq!(a.status, Status::WaitingInput);
        let idle = json!({"message": "Claude is waiting for your input", "notification_type": "idle_prompt"});
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::Notification, 2, idle),
            Origin::Agent,
            2,
        );
        assert_eq!(a.status, Status::WaitingInput);
        assert!(matches!(out, Outcome::Applied { effects, .. } if effects.is_empty()));
        // An explicit report still blocks.
        apply_report(
            &mut a,
            &rep(ReportEvent::Blocked, 3, Value::Null),
            Origin::Agent,
            3,
        );
        assert_eq!(a.status, Status::Blocked);
    }

    #[test]
    fn screen_working_does_not_clear_real_block() {
        let mut a = running();
        apply_report(
            &mut a,
            &rep(ReportEvent::Notification, 1, Value::Null),
            Origin::Agent,
            1,
        );
        assert_eq!(a.status, Status::Blocked);
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::Working, 1, Value::Null),
            Origin::Daemon,
            2,
        );
        assert_eq!(out, Outcome::Stale);
        assert_eq!(a.status, Status::Blocked);
    }

    #[test]
    fn dormant_agents_are_not_revived() {
        let mut a = running();
        a.fire(Trigger::Pause, 1);
        let out = apply_report(
            &mut a,
            &rep(ReportEvent::SessionStart, 1, Value::Null),
            Origin::Agent,
            2,
        );
        assert_eq!(out, Outcome::Dormant);
        assert_eq!(a.status, Status::Paused);
        assert_eq!(a.last_seq, 1);
    }

    #[test]
    fn stop_with_question_waits_for_input() {
        let mut a = running();
        let out = apply_report(
            &mut a,
            &rep(
                ReportEvent::Stop,
                1,
                json!({"message": "Postgres or SQLite?"}),
            ),
            Origin::Agent,
            1,
        );
        assert!(matches!(
            out,
            Outcome::Applied {
                to: Status::WaitingInput,
                ..
            }
        ));
    }

    #[test]
    fn session_and_resume_cmd_recorded_and_released() {
        let mut a = running();
        apply_report(
            &mut a,
            &rep(
                ReportEvent::SessionStart,
                1,
                json!({"session_id": "s-1", "resumeCmd": "claude -r s-1"}),
            ),
            Origin::Agent,
            1,
        );
        assert_eq!(a.session.as_deref(), Some("s-1"));
        assert_eq!(a.resume_cmd.as_deref(), Some("claude -r s-1"));
        apply_report(
            &mut a,
            &rep(ReportEvent::Release, 2, Value::Null),
            Origin::Agent,
            2,
        );
        assert_eq!(a.session, None);
        assert_eq!(a.resume_cmd, None);
    }

    #[test]
    fn slash_commands_are_not_user_prompts() {
        let mut a = running();
        a.fire(Trigger::StoppedIdle, 0);
        let out = apply_report(
            &mut a,
            &rep(
                ReportEvent::UserPromptSubmit,
                1,
                json!({"prompt": "/xherd:next"}),
            ),
            Origin::Agent,
            1,
        );
        let Outcome::Applied { effects, .. } = out else {
            panic!()
        };
        assert_eq!(effects, vec![Effect::DeliveryAck]);
    }

    fn any_event() -> impl Strategy<Value = ReportEvent> {
        prop_oneof![
            Just(ReportEvent::SessionStart),
            Just(ReportEvent::UserPromptSubmit),
            Just(ReportEvent::PostToolUse),
            Just(ReportEvent::Notification),
            Just(ReportEvent::Stop),
            Just(ReportEvent::Working),
            Just(ReportEvent::Idle),
            Just(ReportEvent::Blocked),
            Just(ReportEvent::Release),
        ]
    }

    proptest! {
        /// Reports alone never take a live agent out of the live set, never move
        /// a dormant one, and sequence numbers only grow.
        #[test]
        fn reports_keep_invariants(
            start_live in any::<bool>(),
            steps in prop::collection::vec((any_event(), 0u64..50, any::<bool>()), 0..60),
        ) {
            let mut a = running();
            if !start_live { a.fire(Trigger::Pause, 0); }
            let initial = a.status;
            for (i, (ev, seq, daemon)) in steps.into_iter().enumerate() {
                let before = (a.last_seq, a.last_daemon_seq);
                let origin = if daemon { Origin::Daemon } else { Origin::Agent };
                apply_report(&mut a, &rep(ev, seq, Value::Null), origin, i as i64);
                prop_assert!(a.last_seq >= before.0 && a.last_daemon_seq >= before.1);
                if start_live {
                    prop_assert!(a.status.is_live());
                } else {
                    prop_assert_eq!(a.status, initial);
                }
                if a.status != Status::Blocked { prop_assert!(!a.screen_blocked); }
            }
        }
    }
}
