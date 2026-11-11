//! TODOs (spec §16.2): reminders stored in the `todos` table, checked on the
//! scheduler's 30 s tick. A TODO that comes due notifies once (kind `todo`)
//! and waits for the user: complete, reschedule, edit or delete. They are not
//! agents, so they never count against the concurrency cap (R-006).

use xherd_core::todo::{self, Todo};

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
    #[serde(default)]
    pub repeat_minutes: Option<u64>,
}

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
        todo::check(&i.title, i.note.as_deref(), i.due, i.repeat_minutes)
            .map_err(|m| err("bad_todo", m))?;
        let t = Todo::new(
            xherd_core::new_id(),
            &i.title,
            i.note.as_deref(),
            i.due,
            i.repeat_minutes,
            self.clock.now(),
        );
        self.store.put_todo(&t).await.map_err(store)?;
        Ok(t)
    }

    /// Replace the user's fields; a new time re-opens it.
    pub async fn update_todo(&self, id: &str, i: TodoInput) -> Result<Todo, ActionError> {
        todo::check(&i.title, i.note.as_deref(), i.due, i.repeat_minutes)
            .map_err(|m| err("bad_todo", m))?;
        let mut t = self.todo(id).await?;
        t.title = i.title.trim().to_string();
        t.note = i
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        t.repeat_minutes = i.repeat_minutes;
        if i.due != t.due {
            t.reschedule(i.due);
        }
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

    /// Every 30 s: TODOs whose time has come need you (one notification each,
    /// unless the kind is muted).
    pub async fn todo_tick(&self) {
        let Ok(list) = self.store.list_todos().await else {
            return;
        };
        let now = self.clock.now();
        let muted = self
            .live_config()
            .muted_notifications
            .iter()
            .any(|k| k == "todo");
        for mut t in list {
            if !t.fire(now) {
                continue;
            }
            if let Err(e) = self.store.put_todo(&t).await {
                tracing::warn!(error = %e, "TODO not saved");
                continue;
            }
            if !muted {
                let _ = self.hub.send(Change::Notify {
                    agent_id: format!("todo:{}", t.id),
                    kind: "todo".into(),
                    text: format!("TODO: {}", t.title),
                });
            }
        }
    }
}
