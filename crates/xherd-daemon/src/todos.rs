//! TODOs (spec §16.2): reminders stored in the `todos` table, checked on the
//! scheduler's 30 s tick. A TODO that comes due notifies once (kind `todo`)
//! and waits for the user: complete, reschedule, edit or delete. They are not
//! agents, so they never count against the concurrency cap (R-006).

use std::time::Duration;

use xherd_core::exec::{Cmd, run};
use xherd_core::todo::{self, CheckOutcome, Fire, Input, Todo};

use crate::support::Change;
use crate::{ActionError, Daemon};

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn store(e: xherd_store::StoreError) -> ActionError {
    err("store", e.to_string())
}

/// What the API takes to create or replace a TODO.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TodoInput {
    pub title: String,
    #[serde(default)]
    pub note: Option<String>,
    pub due: i64,
    /// Minutes between repeats, or between checks of a condition.
    #[serde(default)]
    pub repeat_minutes: Option<u64>,
    /// A condition to check at each due time (spec §16.2).
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl TodoInput {
    fn input(&self) -> Input<'_> {
        Input {
            title: &self.title,
            note: self.note.as_deref(),
            due: self.due,
            repeat_minutes: self.repeat_minutes,
            condition: self.condition.as_deref(),
            model: self.model.as_deref(),
        }
    }
}

/// A check run gets as long as the spec says a silent session may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(120);

/// The rules of a check run (appended to the runtime's system prompt).
const CHECK_SYSTEM: &str = "You check whether a condition is met. You are strictly read-only: never create, edit or delete anything and never run a command that changes anything; use only the tools you are given. Find out what is needed, then end your answer with a final line of exactly this form, met being true or false: XHERD_RESULT {\"met\": true, \"summary\": \"one short sentence\"}";

impl Daemon {
    pub async fn todos(&self) -> Result<Vec<Todo>, ActionError> {
        self.store.list_todos().await.map_err(store)
    }

    async fn todo(&self, id: &str) -> Result<Todo, ActionError> {
        self.todos()
            .await?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| err("not_found", format!("no TODO {id}")))
    }

    pub async fn create_todo(&self, i: TodoInput) -> Result<Todo, ActionError> {
        todo::check(&i.input()).map_err(|m| err("bad_todo", m))?;
        let t = Todo::new(xherd_core::new_id(), &i.input(), self.clock.now());
        self.store.put_todo(&t).await.map_err(store)?;
        Ok(t)
    }

    /// Replace the user's fields; a new time re-opens it.
    pub async fn update_todo(&self, id: &str, i: TodoInput) -> Result<Todo, ActionError> {
        todo::check(&i.input()).map_err(|m| err("bad_todo", m))?;
        let mut t = self.todo(id).await?;
        t.edit(&i.input());
        self.store.put_todo(&t).await.map_err(store)?;
        Ok(t)
    }

    pub async fn delete_todo(&self, id: &str) -> Result<(), ActionError> {
        if self.store.delete_todo(id).await.map_err(store)? {
            Ok(())
        } else {
            Err(err("not_found", format!("no TODO {id}")))
        }
    }

    pub async fn complete_todo(&self, id: &str) -> Result<Todo, ActionError> {
        let mut t = self.todo(id).await?;
        t.complete(self.clock.now());
        self.store.put_todo(&t).await.map_err(store)?;
        Ok(t)
    }

    pub async fn reschedule_todo(&self, id: &str, due: i64) -> Result<Todo, ActionError> {
        if due <= 0 {
            return Err(err("bad_todo", "say when it is due"));
        }
        let mut t = self.todo(id).await?;
        t.reschedule(due);
        self.store.put_todo(&t).await.map_err(store)?;
        Ok(t)
    }

    fn todo_muted(&self) -> bool {
        self.live_config()
            .muted_notifications
            .iter()
            .any(|k| k == "todo")
    }

    /// The notification for a TODO that needs you (the title only: what a
    /// check said is model output and stays on the page).
    fn todo_notify(&self, t: &Todo, text: String) {
        if !self.todo_muted() {
            let _ = self.hub.send(Change::Notify {
                agent_id: format!("todo:{}", t.id),
                kind: "todo".into(),
                text,
            });
        }
    }

    /// Every 30 s: TODOs whose time has come need you (one notification each,
    /// unless the kind is muted) or are checked; lost checks become errors.
    pub async fn todo_tick(&self) {
        let Ok(list) = self.store.list_todos().await else {
            return;
        };
        let now = self.clock.now();
        for mut t in list {
            let lost = t.expire_check(now);
            let fire = t.fire(now);
            if !lost && fire == Fire::Nothing {
                continue;
            }
            if let Err(e) = self.store.put_todo(&t).await {
                tracing::warn!(error = %e, "TODO not saved");
                continue;
            }
            if lost {
                self.todo_notify(&t, format!("TODO check failed: {}", t.title));
            }
            match fire {
                Fire::Due => self.todo_notify(&t, format!("TODO: {}", t.title)),
                Fire::Check => self.spawn_check(t),
                Fire::Nothing => {}
            }
        }
    }

    /// "Check now" (spec §16.2).
    pub async fn check_todo(&self, id: &str) -> Result<Todo, ActionError> {
        let mut t = self.todo(id).await?;
        if !t.start_check(self.clock.now()) {
            return Err(err(
                "not_checkable",
                "only a condition that is not being checked can be checked now",
            ));
        }
        self.store.put_todo(&t).await.map_err(store)?;
        self.spawn_check(t.clone());
        Ok(t)
    }

    fn spawn_check(&self, t: Todo) {
        let d = self.clone();
        tokio::spawn(async move {
            let (outcome, summary) = d.run_check(&t).await;
            // The user may have edited or deleted it meanwhile.
            let Ok(mut cur) = d.todo(&t.id).await else {
                return;
            };
            let needs_you = cur.finish_check(d.clock.now(), outcome, summary);
            if let Err(e) = d.store.put_todo(&cur).await {
                tracing::warn!(error = %e, "TODO check result not saved");
                return;
            }
            if needs_you {
                let text = if outcome == CheckOutcome::Failed {
                    format!("TODO check failed: {}", cur.title)
                } else {
                    format!("TODO: {}", cur.title)
                };
                d.todo_notify(&cur, text);
            }
        });
    }

    /// One read-only run of the default runtime, as the agents' user, in the
    /// workspace. Anything but a clear answer is `Unclear` (the user looks);
    /// a run that could not happen is `Failed`.
    async fn run_check(&self, t: &Todo) -> (CheckOutcome, Option<String>) {
        let fail = |m: String| (CheckOutcome::Failed, Some(m));
        let runtime = xherd_flow::approve::DEFAULT_RUNTIME;
        if !self.plugin_enabled(runtime) {
            return fail(format!("the {runtime} plugin is switched off"));
        }
        let Some(check) = self
            .plugins
            .get(runtime)
            .and_then(|p| xherd_plugin::runtime::Terminal::of(&p.manifest).ok())
            .and_then(|term| term.check)
        else {
            return fail(format!("the {runtime} plugin has no check command"));
        };
        let Some(condition) = t.condition.as_deref() else {
            return fail("this TODO has no condition".into());
        };
        let argv = check.command(
            &format!("Condition to check: {condition}"),
            CHECK_SYSTEM,
            t.model.as_deref(),
        );
        let root = self.live_config().workspace_root();
        let cmd = Cmd::new(argv)
            .cwd(if root.is_dir() { root } else { ".".into() })
            .as_agent()
            .timeout(CHECK_TIMEOUT);
        match run(cmd).await {
            Err(e) => fail(e.to_string()),
            Ok(o) if o.timed_out => (
                CheckOutcome::Unclear,
                Some(format!("no answer in {} seconds", CHECK_TIMEOUT.as_secs())),
            ),
            Ok(o) if o.code != Some(0) => {
                let why: String = o
                    .stderr
                    .lines()
                    .next()
                    .unwrap_or("it failed")
                    .chars()
                    .take(200)
                    .collect();
                fail(format!("the run ended with code {:?}: {why}", o.code))
            }
            Ok(o) => match todo::parse_result(&o.stdout_str()) {
                Some((true, s)) => (CheckOutcome::Met, s),
                Some((false, s)) => (CheckOutcome::NotMet, s),
                None => (
                    CheckOutcome::Unclear,
                    Some("it ended without the answer line".into()),
                ),
            },
        }
    }
}
