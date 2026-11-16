//! SQLite store (design §3.3): WAL, one writer thread owning the only
//! read-write connection, a small pool of read-only connections, forward-only
//! migrations run at open after a `VACUUM INTO` copy (R-1305).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use serde_json::Value;
use xherd_core::{Job, Millis};

mod schema;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("migration failed (the previous database is kept as {backup:?}): {source}")]
    Migration {
        backup: Option<PathBuf>,
        source: rusqlite_migration::Error,
    },
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store writer stopped")]
    Closed,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

type WriteFn = Box<dyn FnOnce(&mut Connection) + Send>;

/// Event kinds that are records, not history: `prune_events` keeps them.
/// `ticket-comment` says a summary was posted on the agent's ticket, which
/// must not be offered again once the retention period has passed. Plain
/// names only (they go into the SQL text).
pub const KEPT_EVENTS: &[&str] = &["ticket-comment"];

/// An event in the agent log; `id` makes replays idempotent.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub job_id: String,
    pub kind: String,
    pub payload: Option<Value>,
    pub at: Millis,
}

/// One row of the `approvals` table (design §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub job_id: String,
    pub file: String,
    pub sha256: String,
    pub at: Millis,
    pub actor: String,
    pub auto: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub id: String,
    pub kind: String,
    pub subject: Option<String>,
    pub scopes: Vec<String>,
    pub revoked: bool,
}

#[derive(Clone)]
pub struct Store {
    writer: std::sync::mpsc::Sender<WriteFn>,
    readers: Arc<Mutex<Vec<Connection>>>,
    path: PathBuf,
}

/// Notes kept per job (the user's own texts; R31).
pub const NOTES_KEPT: i64 = 50;

const READERS: usize = 4;

/// Writes a database as migration 1 left it (tables `agents`, column
/// `agent_id`) holding `job`, one `user.pause` event and one approval of
/// `spec.md` with hash `h`. For tests of the migration to jobs (issue #141).
#[doc(hidden)]
pub fn write_version_1_database(db: &Path, job: &Job) -> Result<()> {
    let mut c = Connection::open(db)?;
    Migrations::new(vec![M::up(schema::MIGRATIONS[0])])
        .to_latest(&mut c)
        .map_err(|source| StoreError::Migration {
            backup: None,
            source,
        })?;
    let data = serde_json::to_string(job).map_err(|e| StoreError::Io(std::io::Error::other(e)))?;
    c.execute(
        "INSERT INTO agents (id, name, status, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            job.id,
            job.name,
            job.status.to_string(),
            job.created_at,
            data
        ],
    )?;
    c.execute(
        "INSERT INTO events (id, agent_id, kind, payload, at) VALUES ('e1', ?1, 'user.pause', NULL, 5)",
        [&job.id],
    )?;
    c.execute(
        "INSERT INTO approvals (agent_id, file, sha256, at, actor, auto) VALUES (?1, 'spec.md', 'h', 5, 'user', 0)",
        [&job.id],
    )?;
    Ok(())
}

fn migrations() -> Migrations<'static> {
    Migrations::new(schema::MIGRATIONS.iter().map(|sql| M::up(sql)).collect())
}

/// `PRAGMA integrity_check` on a database file, opened read-only (R-1404):
/// `Ok(())` when it says `ok`, else the first complaint.
pub fn check_integrity(db: &Path) -> Result<()> {
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let first: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if first == "ok" {
        Ok(())
    } else {
        Err(StoreError::Io(std::io::Error::other(format!(
            "integrity_check: {first}"
        ))))
    }
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")
}

impl Store {
    /// Open (creating if needed) and migrate. `backups` receives
    /// `pre-migrate-<xherd version>-from-<schema>.db` before any migration of an existing database.
    pub fn open(db: &Path, backups: &Path) -> Result<Store> {
        Self::open_with(db, backups, migrations())
    }

    fn open_with(db: &Path, backups: &Path, migs: Migrations<'static>) -> Result<Store> {
        let mut conn = Connection::open(db)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        configure(&conn)?;

        let pending = migs
            .pending_migrations(&conn)
            .map_err(|source| StoreError::Migration {
                backup: None,
                source,
            })?;
        let current: usize = match migs.current_version(&conn) {
            Ok(rusqlite_migration::SchemaVersion::Inside(v)) => v.get(),
            _ => 0,
        };
        let mut backup = None;
        if pending > 0 && current > 0 {
            std::fs::create_dir_all(backups)?;
            let f = backups.join(format!(
                "pre-migrate-{}-from-{current}.db",
                xherd_core::VERSION
            ));
            let _ = std::fs::remove_file(&f);
            conn.execute("VACUUM INTO ?1", [f.to_string_lossy()])?;
            backup = Some(f);
        }
        migs.to_latest(&mut conn)
            .map_err(|source| StoreError::Migration {
                backup: backup.clone(),
                source,
            })?;
        if pending > 0 {
            tracing::info!(from = current, applied = pending, "database migrated");
        }

        let (tx, rx) = std::sync::mpsc::channel::<WriteFn>();
        std::thread::Builder::new()
            .name("xherd-store-writer".into())
            .spawn(move || {
                let mut conn = conn;
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })?;

        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            let r = Connection::open_with_flags(
                db,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            configure(&r)?;
            readers.push(r);
        }
        Ok(Store {
            writer: tx,
            readers: Arc::new(Mutex::new(readers)),
            path: db.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A consistent copy of the live database into `out` (`VACUUM INTO`, so
    /// it is safe while the daemon writes; R-1401). `out` must not exist.
    pub async fn snapshot(&self, out: &Path) -> Result<()> {
        let out = out.to_path_buf();
        self.write(move |c| {
            c.execute("VACUUM INTO ?1", [out.to_string_lossy()])?;
            Ok(())
        })
        .await
    }

    /// Run `f` on the writer connection; requests are served in order.
    pub async fn write<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.writer
            .send(Box::new(move |c| {
                let _ = tx.send(f(c));
            }))
            .map_err(|_| StoreError::Closed)?;
        rx.await.map_err(|_| StoreError::Closed)?
    }

    /// Run `f` on a read-only connection in the blocking pool.
    pub async fn read<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let readers = self.readers.clone();
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let pooled = readers.lock().expect("reader pool").pop();
            let conn = match pooled {
                Some(c) => c,
                None => {
                    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                    configure(&c)?;
                    c
                }
            };
            let out = f(&conn);
            let mut pool = readers.lock().expect("reader pool");
            if pool.len() < READERS {
                pool.push(conn);
            }
            out
        })
        .await
        .map_err(|_| StoreError::Closed)?
    }

    // ---- agents ------------------------------------------------------------

    pub async fn put_job(&self, a: &Job) -> Result<()> {
        let data = serde_json::to_string(a)?;
        let (id, name, status, created) = (
            a.id.clone(),
            a.name.clone(),
            a.status.to_string(),
            a.created_at,
        );
        self.write(move |c| {
            c.execute(
                "INSERT INTO jobs (id, name, status, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, status = excluded.status, data = excluded.data",
                params![id, name, status, created, data],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn get_job(&self, id: &str) -> Result<Option<Job>> {
        let id = id.to_string();
        self.read(move |c| {
            let data: Option<String> = c
                .query_row("SELECT data FROM jobs WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            Ok(data
                .map(|d| serde_json::from_str::<Job>(&d))
                .transpose()?
                .map(Job::upgrade))
        })
        .await
    }

    pub async fn list_jobs(&self) -> Result<Vec<Job>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM jobs ORDER BY created_at, id")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for d in rows {
                out.push(serde_json::from_str::<Job>(&d?)?.upgrade());
            }
            Ok(out)
        })
        .await
    }

    pub async fn delete_job(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.write(move |c| {
            let tx = c.transaction()?;
            for t in ["events", "notes", "approvals", "reviewed", "diff_viewed"] {
                tx.execute(&format!("DELETE FROM {t} WHERE job_id = ?1"), [&id])?;
            }
            tx.execute("DELETE FROM jobs WHERE id = ?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    // ---- approvals -----------------------------------------------------------

    /// The ledger approves `file` at exactly this content (R-107).
    pub async fn approved(&self, job_id: &str, file: &str, sha256: &str) -> Result<bool> {
        let (a, f, h) = (job_id.to_string(), file.to_string(), sha256.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT 1 FROM approvals WHERE job_id = ?1 AND file = ?2 AND sha256 = ?3",
                params![a, f, h],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })
        .await
    }

    /// Record an approval; an identical (agent, file, hash) row is kept.
    pub async fn insert_approval(&self, r: Approval) -> Result<()> {
        self.write(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO approvals (job_id, file, sha256, at, actor, auto, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![r.job_id, r.file, r.sha256, r.at, r.actor, r.auto, r.reason],
            )?;
            Ok(())
        })
        .await
    }

    /// Files with at least one approval for this agent.
    pub async fn approved_files(&self, job_id: &str) -> Result<BTreeSet<String>> {
        let a = job_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare("SELECT DISTINCT file FROM approvals WHERE job_id = ?1")?;
            let rows = st.query_map([a], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<BTreeSet<String>>>()?)
        })
        .await
    }

    /// The user dismissed a starter `path` (`AGENTS.md`) for `repo` (spec
    /// §16.6): agents stop proposing one there.
    pub async fn put_memory_declined(&self, repo: &str, path: &str) -> Result<()> {
        let (r, p) = (repo.to_string(), path.to_string());
        self.write(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO memory_declined (repo, path) VALUES (?1, ?2)",
                params![r, p],
            )?;
            Ok(())
        })
        .await
    }

    /// A text the user wrote to a job (Send, answer, Ask, change request),
    /// kept for the agent draft of a job without an agent (R31). Newest
    /// notes beyond `NOTES_KEPT` per job are dropped.
    pub async fn add_note(&self, job_id: &str, text: &str, at: i64) -> Result<()> {
        let (j, t) = (job_id.to_string(), text.to_string());
        self.write(move |c| {
            c.execute(
                "INSERT INTO notes (job_id, text, at) VALUES (?1, ?2, ?3)",
                params![j, t, at],
            )?;
            c.execute(
                "DELETE FROM notes WHERE job_id = ?1 AND id NOT IN \
                 (SELECT id FROM notes WHERE job_id = ?1 ORDER BY id DESC LIMIT ?2)",
                params![j, NOTES_KEPT],
            )?;
            Ok(())
        })
        .await
    }

    /// A job's notes, oldest first.
    pub async fn notes(&self, job_id: &str) -> Result<Vec<(String, i64)>> {
        let j = job_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare("SELECT text, at FROM notes WHERE job_id = ?1 ORDER BY id")?;
            let rows = st.query_map([j], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .await
    }

    pub async fn memory_declined(&self, repo: &str, path: &str) -> Result<bool> {
        let (r, p) = (repo.to_string(), path.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT 1 FROM memory_declined WHERE repo = ?1 AND path = ?2",
                params![r, p],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })
        .await
    }

    /// The text the user approved last, for "changed since you reviewed".
    pub async fn put_reviewed(&self, job_id: &str, file: &str, text: &str) -> Result<()> {
        let (a, f, t) = (job_id.to_string(), file.to_string(), text.to_string());
        self.write(move |c| {
            c.execute(
                "INSERT INTO reviewed (job_id, file, text) VALUES (?1, ?2, ?3)
                 ON CONFLICT(job_id, file) DO UPDATE SET text = excluded.text",
                params![a, f, t],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn reviewed(&self, job_id: &str, file: &str) -> Result<Option<String>> {
        let (a, f) = (job_id.to_string(), file.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT text FROM reviewed WHERE job_id = ?1 AND file = ?2",
                params![a, f],
                |r| r.get(0),
            )
            .optional()?)
        })
        .await
    }

    /// The "viewed" mark of a file in the Changes panel: `fingerprint` is the
    /// UI's fingerprint of the file's diff, so a later change un-marks it.
    /// `None` removes the mark.
    pub async fn put_viewed(
        &self,
        job_id: &str,
        path: &str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        let (a, p, f) = (
            job_id.to_string(),
            path.to_string(),
            fingerprint.map(str::to_string),
        );
        self.write(move |c| {
            match f {
                Some(f) => c.execute(
                    "INSERT INTO diff_viewed (job_id, path, fingerprint) VALUES (?1, ?2, ?3)
                     ON CONFLICT(job_id, path) DO UPDATE SET fingerprint = excluded.fingerprint",
                    params![a, p, f],
                )?,
                None => c.execute(
                    "DELETE FROM diff_viewed WHERE job_id = ?1 AND path = ?2",
                    params![a, p],
                )?,
            };
            Ok(())
        })
        .await
    }

    /// Path → fingerprint of every file the user marked viewed.
    pub async fn viewed(&self, job_id: &str) -> Result<std::collections::BTreeMap<String, String>> {
        let a = job_id.to_string();
        self.read(move |c| {
            let mut st =
                c.prepare("SELECT path, fingerprint FROM diff_viewed WHERE job_id = ?1")?;
            let rows = st.query_map([a], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    // ---- todos ---------------------------------------------------------------

    pub async fn put_todo(&self, t: &xherd_core::todo::Todo) -> Result<()> {
        let data = serde_json::to_string(t)?;
        let (id, created) = (t.id.clone(), t.created_at);
        self.write(move |c| {
            c.execute(
                "INSERT INTO todos (id, created_at, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                params![id, created, data],
            )?;
            Ok(())
        })
        .await
    }

    /// Oldest first; rows that no longer parse are skipped.
    pub async fn list_todos(&self) -> Result<Vec<xherd_core::todo::Todo>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM todos ORDER BY created_at, id")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(t) = serde_json::from_str(&row?) {
                    out.push(t);
                }
            }
            Ok(out)
        })
        .await
    }

    /// Whether a row was removed.
    pub async fn delete_todo(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.write(move |c| Ok(c.execute("DELETE FROM todos WHERE id = ?1", [id])? == 1))
            .await
    }

    // ---- profiles --------------------------------------------------------------

    pub async fn put_profile(&self, p: &xherd_core::profile::Profile) -> Result<()> {
        let data = serde_json::to_string(p)?;
        let id = p.id.clone();
        self.write(move |c| {
            c.execute(
                "INSERT INTO profiles (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                params![id, data],
            )?;
            Ok(())
        })
        .await
    }

    /// In creation order (rowid); rows that no longer parse are skipped.
    pub async fn list_profiles(&self) -> Result<Vec<xherd_core::profile::Profile>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM profiles ORDER BY rowid")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(p) = serde_json::from_str(&row?) {
                    out.push(p);
                }
            }
            Ok(out)
        })
        .await
    }

    /// Whether a row was removed.
    pub async fn delete_profile(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.write(move |c| Ok(c.execute("DELETE FROM profiles WHERE id = ?1", [id])? == 1))
            .await
    }

    // ---- saved agents ------------------------------------------------------------

    /// Write the agent and its `agent_versions` row for `def.version` in one
    /// transaction (R11). A name used by another agent is an error.
    pub async fn put_agent_def(&self, def: &xherd_core::agent_def::AgentDef) -> Result<()> {
        let data = serde_json::to_string(def)?;
        let (id, name, version) = (def.id.clone(), def.name.clone(), def.version);
        let (archived, at) = (def.archived_at, def.updated_at);
        self.write(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO agents (id, name, archived_at, data) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                   archived_at = excluded.archived_at, data = excluded.data",
                params![id, name, archived, data],
            )?;
            tx.execute(
                "INSERT INTO agent_versions (agent_id, version, at, data) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id, version) DO UPDATE SET at = excluded.at, data = excluded.data",
                params![id, version, at, data],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn get_agent_def(&self, id: &str) -> Result<Option<xherd_core::agent_def::AgentDef>> {
        let id = id.to_string();
        self.read(move |c| {
            let data: Option<String> = c
                .query_row("SELECT data FROM agents WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
        })
        .await
    }

    /// In creation order (rowid); rows that no longer parse are skipped.
    pub async fn list_agent_defs(&self) -> Result<Vec<xherd_core::agent_def::AgentDef>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM agents ORDER BY rowid")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(a) = serde_json::from_str(&row?) {
                    out.push(a);
                }
            }
            Ok(out)
        })
        .await
    }

    /// Every stored version of an agent, newest first.
    pub async fn agent_versions(&self, id: &str) -> Result<Vec<xherd_core::agent_def::AgentDef>> {
        let id = id.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT data FROM agent_versions WHERE agent_id = ?1 ORDER BY version DESC",
            )?;
            let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(a) = serde_json::from_str(&row?) {
                    out.push(a);
                }
            }
            Ok(out)
        })
        .await
    }

    /// Remove the agent, its versions and its memory; whether the agent existed.
    pub async fn delete_agent_def(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.write(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM agent_versions WHERE agent_id = ?1", [&id])?;
            tx.execute("DELETE FROM agent_memory WHERE agent_id = ?1", [&id])?;
            let n = tx.execute("DELETE FROM agents WHERE id = ?1", [&id])?;
            tx.commit()?;
            Ok(n == 1)
        })
        .await
    }

    /// The agent's memory text and its version.
    pub async fn agent_memory(&self, id: &str) -> Result<Option<(String, u32)>> {
        let id = id.to_string();
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT text, version FROM agent_memory WHERE agent_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await
    }

    /// Replace the memory; its version grows by one (1 for the first write).
    /// Returns the new version.
    pub async fn put_agent_memory(&self, id: &str, text: &str, now: Millis) -> Result<u32> {
        let (id, text) = (id.to_string(), text.to_string());
        self.write(move |c| {
            c.execute(
                "INSERT INTO agent_memory (agent_id, text, updated_at, version) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(agent_id) DO UPDATE SET text = excluded.text,
                   updated_at = excluded.updated_at, version = version + 1",
                params![id, text, now],
            )?;
            Ok(c.query_row(
                "SELECT version FROM agent_memory WHERE agent_id = ?1",
                [id],
                |r| r.get(0),
            )?)
        })
        .await
    }

    // ---- work log ------------------------------------------------------------

    /// Add `ms` to the work log of `ticket` on `day` (`YYYY-MM-DD`, UTC);
    /// `kind` is `agent` (time in `running`) or `user` (time on the agent page).
    pub async fn add_worklog(&self, day: &str, ticket: &str, kind: &str, ms: u64) -> Result<()> {
        let (d, t, k, ms) = (
            day.to_string(),
            ticket.to_string(),
            kind.to_string(),
            ms as i64,
        );
        self.write(move |c| {
            c.execute(
                "INSERT INTO worklog (day, ticket_key, kind, ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(day, ticket_key, kind) DO UPDATE SET ms = ms + excluded.ms",
                params![d, t, k, ms],
            )?;
            Ok(())
        })
        .await
    }

    /// `(day, ticket, kind, ms)` from `since_day` on, newest day first.
    pub async fn worklog(&self, since_day: &str) -> Result<Vec<(String, String, String, u64)>> {
        let since = since_day.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT day, ticket_key, kind, ms FROM worklog WHERE day >= ?1
                 ORDER BY day DESC, ticket_key, kind",
            )?;
            let rows = st.query_map([since], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as u64))
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// Every event of `kind`, oldest first (delivery metrics).
    pub async fn events_of_kind(&self, kind: &str) -> Result<Vec<Event>> {
        let k = kind.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, job_id, kind, payload, at FROM events WHERE kind = ?1 ORDER BY at, id",
            )?;
            let rows = st.query_map([k], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, job_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event {
                    id,
                    job_id,
                    kind,
                    payload,
                    at,
                });
            }
            Ok(out)
        })
        .await
    }

    // ---- usage ---------------------------------------------------------------

    /// Add tokens to `scope`'s total for `day` (`YYYY-MM-DD`, UTC).
    pub async fn add_usage(&self, scope: &str, day: &str, tokens: u64) -> Result<()> {
        let (s, d, t) = (scope.to_string(), day.to_string(), tokens as i64);
        self.write(move |c| {
            c.execute(
                "INSERT INTO budgets (scope, day, tokens) VALUES (?1, ?2, ?3)
                 ON CONFLICT(scope, day) DO UPDATE SET tokens = tokens + excluded.tokens",
                params![s, d, t],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn usage(&self, scope: &str, day: &str) -> Result<u64> {
        let (s, d) = (scope.to_string(), day.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT tokens FROM budgets WHERE scope = ?1 AND day = ?2",
                params![s, d],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64)
        })
        .await
    }

    // ---- installs ----------------------------------------------------------

    /// Hash of what `xherd install` last wrote at `path` (design §6.7).
    pub async fn install_sha(
        &self,
        runtime: &str,
        pack: &str,
        path: &str,
    ) -> Result<Option<String>> {
        let (r, p, f) = (runtime.to_string(), pack.to_string(), path.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT sha256 FROM installs WHERE runtime = ?1 AND pack = ?2 AND path = ?3",
                params![r, p, f],
                |r| r.get(0),
            )
            .optional()?)
        })
        .await
    }

    pub async fn put_install(
        &self,
        runtime: &str,
        pack: &str,
        path: &str,
        sha256: &str,
    ) -> Result<()> {
        let (r, p, f, h) = (
            runtime.to_string(),
            pack.to_string(),
            path.to_string(),
            sha256.to_string(),
        );
        self.write(move |c| {
            c.execute(
                "INSERT INTO installs (runtime, pack, path, sha256) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(runtime, pack, path) DO UPDATE SET sha256 = excluded.sha256",
                params![r, p, f, h],
            )?;
            Ok(())
        })
        .await
    }

    // ---- events ------------------------------------------------------------

    /// Insert unless an event with this id exists. Returns whether it was new.
    pub async fn insert_event(&self, e: Event) -> Result<bool> {
        let payload = e.payload.as_ref().map(serde_json::to_string).transpose()?;
        self.write(move |c| {
            let n = c.execute(
                "INSERT OR IGNORE INTO events (id, job_id, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.id, e.job_id, e.kind, payload, e.at],
            )?;
            Ok(n == 1)
        })
        .await
    }

    pub async fn event_exists(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.read(move |c| {
            Ok(
                c.query_row("SELECT 1 FROM events WHERE id = ?1", [id], |_| Ok(()))
                    .optional()?
                    .is_some(),
            )
        })
        .await
    }

    /// `job_id|at|gate` of every auto-approval marked seen (events of kind
    /// `review-seen`, pruned with the approvals themselves).
    pub async fn reviews_seen(&self) -> Result<std::collections::HashSet<String>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT id FROM events WHERE kind = 'review-seen'")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
    }

    /// Most recent first.
    pub async fn events(&self, job_id: &str, limit: usize) -> Result<Vec<Event>> {
        let id = job_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, job_id, kind, payload, at FROM events WHERE job_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![id, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?, r.get(4)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, job_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event { id, job_id, kind, payload, at });
            }
            Ok(out)
        })
        .await
    }

    /// Automatic approvals (`approval` events with `auto: true`), newest
    /// first, across agents (R-111).
    pub async fn auto_approvals(&self, limit: usize) -> Result<Vec<Event>> {
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, job_id, kind, payload, at FROM events
                 WHERE kind = 'approval' AND json_extract(payload, '$.auto') = 1
                 ORDER BY at DESC, id DESC LIMIT ?1",
            )?;
            let rows = st.query_map(params![limit as i64], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, job_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event {
                    id,
                    job_id,
                    kind,
                    payload,
                    at,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Delete events older than `before`, except the kinds in [`KEPT_EVENTS`].
    /// Returns how many were removed.
    pub async fn prune_events(&self, before: Millis) -> Result<usize> {
        self.write(move |c| {
            let kept = KEPT_EVENTS
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM events WHERE at < ?1 AND kind NOT IN ({kept})");
            Ok(c.execute(&sql, [before])?)
        })
        .await
    }

    /// Whether agent `job_id` has an event of `kind`.
    pub async fn agent_has_event(&self, job_id: &str, kind: &str) -> Result<bool> {
        let (a, k) = (job_id.to_string(), kind.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE job_id = ?1 AND kind = ?2)",
                [a, k],
                |r| r.get(0),
            )?)
        })
        .await
    }

    // ---- tokens (only hashes are stored) -------------------------------------

    pub async fn insert_token(
        &self,
        id: &str,
        kind: &str,
        subject: Option<&str>,
        scopes: &[&str],
        token_hash: &str,
        now: Millis,
    ) -> Result<()> {
        let (id, kind, subject, hash) = (
            id.to_string(),
            kind.to_string(),
            subject.map(str::to_string),
            token_hash.to_string(),
        );
        let scopes = serde_json::to_string(scopes)?;
        self.write(move |c| {
            c.execute(
                "INSERT INTO tokens (id, kind, subject, scopes, token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, kind, subject, scopes, hash, now],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn find_token(&self, token_hash: &str) -> Result<Option<TokenRow>> {
        let hash = token_hash.to_string();
        self.read(move |c| {
            let row = c
                .query_row(
                    "SELECT id, kind, subject, scopes, revoked_at IS NOT NULL FROM tokens WHERE token_hash = ?1",
                    [hash],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, bool>(4)?,
                        ))
                    },
                )
                .optional()?;
            Ok(match row {
                Some((id, kind, subject, scopes, revoked)) => Some(TokenRow {
                    id,
                    kind,
                    subject,
                    scopes: serde_json::from_str(&scopes)?,
                    revoked,
                }),
                None => None,
            })
        })
        .await
    }

    pub async fn touch_token(&self, id: &str, now: Millis) -> Result<()> {
        let id = id.to_string();
        self.write(move |c| {
            c.execute(
                "UPDATE tokens SET last_seen = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
        .await
    }

    /// Revoke every live token of `kind` for `subject` (e.g. an agent's session tokens).
    pub async fn revoke_tokens(&self, kind: &str, subject: &str, now: Millis) -> Result<usize> {
        let (kind, subject) = (kind.to_string(), subject.to_string());
        self.write(move |c| {
            Ok(c.execute(
                "UPDATE tokens SET revoked_at = ?1 WHERE kind = ?2 AND subject = ?3 AND revoked_at IS NULL",
                params![now, kind, subject],
            )?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xherd_core::Status;

    fn open() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("x.db"), &d.path().join("backups")).unwrap();
        (d, s)
    }

    #[test]
    fn migrations_are_valid() {
        migrations().validate().unwrap();
    }

    #[tokio::test]
    async fn viewed_marks_are_set_replaced_and_cleared() {
        let (_d, s) = open();
        s.put_viewed("a", "x.rs", Some("1")).await.unwrap();
        s.put_viewed("a", "y.rs", Some("2")).await.unwrap();
        s.put_viewed("a", "x.rs", Some("3")).await.unwrap();
        s.put_viewed("b", "x.rs", Some("9")).await.unwrap();
        s.put_viewed("a", "y.rs", None).await.unwrap();
        let v = s.viewed("a").await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v["x.rs"], "3");
    }

    /// Migration 2 (issue #141): a version-1 database keeps every row under
    /// the new table and column names.
    #[tokio::test]
    async fn migration_2_renames_agents_to_jobs_and_keeps_rows() {
        let d = tempfile::tempdir().unwrap();
        let (db, backups) = (d.path().join("x.db"), d.path().join("backups"));
        let job = Job::new("SC-1", 1);
        {
            let mut c = Connection::open(&db).unwrap();
            let v1 = Migrations::new(vec![M::up(schema::MIGRATIONS[0])]);
            v1.to_latest(&mut c).unwrap();
            let data = serde_json::to_string(&job).unwrap();
            let id = &job.id;
            for sql in [
                format!(
                    "INSERT INTO agents VALUES ('{id}', 'SC-1', 'queued', 1, '{}')",
                    data.replace('\'', "''")
                ),
                format!("INSERT INTO events VALUES ('e1', '{id}', 'report.stop', NULL, 5)"),
                format!("INSERT INTO notes VALUES (1, '{id}', 'n', 5)"),
                format!(
                    "INSERT INTO approvals VALUES ('{id}', 'spec.md', 'h', 5, 'user', 0, NULL)"
                ),
                format!("INSERT INTO reviewed VALUES ('{id}', 'spec.md', 'text')"),
                format!("INSERT INTO diff_viewed VALUES ('{id}', 'a.rs', 'fp')"),
            ] {
                c.execute(&sql, []).unwrap();
            }
        }
        let s = Store::open(&db, &backups).unwrap();
        assert!(
            backups.read_dir().unwrap().next().is_some(),
            "pre-migrate copy"
        );
        assert_eq!(s.get_job(&job.id).await.unwrap(), Some(job.clone()));
        let ev = s.events(&job.id, 10).await.unwrap();
        assert_eq!((ev.len(), ev[0].job_id.as_str()), (1, job.id.as_str()));
        assert!(s.approved(&job.id, "spec.md", "h").await.unwrap());
        assert_eq!(
            s.reviewed(&job.id, "spec.md").await.unwrap().as_deref(),
            Some("text")
        );
        assert_eq!(s.viewed(&job.id).await.unwrap()["a.rs"], "fp");
        let c = Connection::open(&db).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE job_id = ?1",
                [&job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        s.delete_job(&job.id).await.unwrap();
        assert!(s.events(&job.id, 10).await.unwrap().is_empty());
    }

    fn agent(id: &str, name: &str, version: u32) -> xherd_core::agent_def::AgentDef {
        use xherd_core::agent_def::AgentDef;
        AgentDef {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            version,
            runtime: "claude-code".into(),
            models: Default::default(),
            fallback_runtime: None,
            autonomy: None,
            permission_mode: None,
            instructions: None,
            job_token_budget: None,
            sensitive_paths: vec![],
            max_diff_lines: None,
            skills: vec![],
            max_live_jobs: 1,
            created_at: 1,
            updated_at: 10 + Millis::from(version),
            archived_at: None,
        }
    }

    /// Migration 4 (issue #141, D26): the dropped keys leave the agent's row
    /// and every version, and the rows still parse as `AgentDef`.
    #[tokio::test]
    async fn migration_4_strips_the_workflow_keys_from_agents_and_versions() {
        let d = tempfile::tempdir().unwrap();
        let (db, backups) = (d.path().join("x.db"), d.path().join("backups"));
        let old = r#"{"id":"a1","name":"rev","version":2,"runtime":"claude-code","workflow":{"pack":"xherd","flow":"bugfix"},"repo":"/r","baseBranch":"main","stageOverrides":{"ship":{"human":true}},"maxDiffLines":5}"#;
        {
            let mut c = Connection::open(&db).unwrap();
            let v3 = Migrations::new(schema::MIGRATIONS[..3].iter().map(|s| M::up(s)).collect());
            v3.to_latest(&mut c).unwrap();
            c.execute("INSERT INTO agents VALUES ('a1','rev',NULL,?1)", [old])
                .unwrap();
            c.execute("INSERT INTO agent_versions VALUES ('a1',2,5,?1)", [old])
                .unwrap();
        }
        let s = Store::open(&db, &backups).unwrap();
        let got = s.get_agent_def("a1").await.unwrap().unwrap();
        assert_eq!((got.name.as_str(), got.max_diff_lines), ("rev", Some(5)));
        let vs = s.agent_versions("a1").await.unwrap();
        assert_eq!((vs.len(), vs[0].version), (1, 2));
        let c = Connection::open(&db).unwrap();
        let data: String = c
            .query_row("SELECT data FROM agent_versions", [], |r| r.get(0))
            .unwrap();
        for key in ["workflow", "repo", "baseBranch", "stageOverrides"] {
            assert!(!data.contains(key), "{key} left in {data}");
        }
    }

    /// Migration 3: the row and its version are written together; delete
    /// removes the agent, its versions and its memory, and nothing else.
    #[tokio::test]
    async fn agents_versions_and_memory() {
        let (_d, s) = open();
        let mut a = agent("a1", "reviewer", 1);
        s.put_agent_def(&a).await.unwrap();
        a.version = 2;
        a.updated_at = 20;
        a.archived_at = Some(30);
        s.put_agent_def(&a).await.unwrap();
        s.put_agent_def(&agent("a2", "other", 1)).await.unwrap();

        assert_eq!(s.get_agent_def("a1").await.unwrap(), Some(a.clone()));
        assert_eq!(s.get_agent_def("nope").await.unwrap(), None);
        let names: Vec<_> = s
            .list_agent_defs()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, ["reviewer", "other"]);
        let vs: Vec<u32> = s
            .agent_versions("a1")
            .await
            .unwrap()
            .iter()
            .map(|d| d.version)
            .collect();
        assert_eq!(vs, [2, 1]);

        // A second agent cannot take a name in use, and nothing of it is kept.
        assert!(s.put_agent_def(&agent("a3", "reviewer", 1)).await.is_err());
        assert!(s.agent_versions("a3").await.unwrap().is_empty());

        assert_eq!(s.agent_memory("a1").await.unwrap(), None);
        assert_eq!(s.put_agent_memory("a1", "one", 5).await.unwrap(), 1);
        assert_eq!(s.put_agent_memory("a1", "two", 6).await.unwrap(), 2);
        assert_eq!(s.agent_memory("a1").await.unwrap(), Some(("two".into(), 2)));
        s.put_agent_memory("a2", "keep", 6).await.unwrap();

        assert!(s.delete_agent_def("a1").await.unwrap());
        assert!(!s.delete_agent_def("a1").await.unwrap());
        assert_eq!(s.get_agent_def("a1").await.unwrap(), None);
        assert!(s.agent_versions("a1").await.unwrap().is_empty());
        assert_eq!(s.agent_memory("a1").await.unwrap(), None);
        assert_eq!(
            s.agent_memory("a2").await.unwrap(),
            Some(("keep".into(), 1))
        );
        assert_eq!(s.agent_versions("a2").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn agent_roundtrip_and_update() {
        let (_d, s) = open();
        let mut a = Job::new("SC-1", 1);
        s.put_job(&a).await.unwrap();
        a.status = Status::Running;
        s.put_job(&a).await.unwrap();
        assert_eq!(s.get_job(&a.id).await.unwrap(), Some(a.clone()));
        assert_eq!(s.list_jobs().await.unwrap().len(), 1);
        s.delete_job(&a.id).await.unwrap();
        assert_eq!(s.get_job(&a.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn events_are_idempotent_and_pruned() {
        let (_d, s) = open();
        let e = Event {
            id: "e1".into(),
            job_id: "a".into(),
            kind: "report.stop".into(),
            payload: None,
            at: 10,
        };
        assert!(s.insert_event(e.clone()).await.unwrap());
        assert!(!s.insert_event(e).await.unwrap());
        assert!(s.event_exists("e1").await.unwrap());
        s.insert_event(Event {
            id: "e2".into(),
            job_id: "a".into(),
            kind: "k".into(),
            payload: Some(serde_json::json!({"x": 1})),
            at: 20,
        })
        .await
        .unwrap();
        let ev = s.events("a", 10).await.unwrap();
        assert_eq!(ev[0].id, "e2");
        assert_eq!(ev[0].payload.as_ref().unwrap()["x"], 1);
        assert_eq!(s.prune_events(15).await.unwrap(), 1);
        assert_eq!(s.events("a", 10).await.unwrap().len(), 1);
        // A record kind outlives the retention period.
        s.insert_event(Event {
            id: "e3".into(),
            job_id: "a".into(),
            kind: "ticket-comment".into(),
            payload: None,
            at: 5,
        })
        .await
        .unwrap();
        assert_eq!(s.prune_events(100).await.unwrap(), 1);
        assert!(s.agent_has_event("a", "ticket-comment").await.unwrap());
        assert!(!s.agent_has_event("b", "ticket-comment").await.unwrap());
        assert!(!s.agent_has_event("a", "k").await.unwrap());
    }

    #[tokio::test]
    async fn tokens_by_hash_and_revocation() {
        let (_d, s) = open();
        s.insert_token("t1", "agent", Some("a1"), &["hooks", "xap"], "h1", 1)
            .await
            .unwrap();
        let t = s.find_token("h1").await.unwrap().unwrap();
        assert_eq!(t.scopes, vec!["hooks", "xap"]);
        assert!(!t.revoked);
        assert_eq!(s.revoke_tokens("agent", "a1", 2).await.unwrap(), 1);
        assert!(s.find_token("h1").await.unwrap().unwrap().revoked);
        assert_eq!(s.find_token("nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn reopen_is_a_noop_and_backs_up_before_migrating() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("x.db");
        let backups = d.path().join("backups");
        let count = |dir: &Path| std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0);
        drop(Store::open(&db, &backups).unwrap());
        drop(Store::open(&db, &backups).unwrap());
        assert_eq!(
            count(&backups),
            0,
            "fresh and up-to-date opens make no copy"
        );

        // A newer release adds a migration: the old file is copied first.
        let mut next: Vec<M> = schema::MIGRATIONS.iter().map(|sql| M::up(sql)).collect();
        next.push(M::up("CREATE TABLE later (x INTEGER);"));
        drop(Store::open_with(&db, &backups, Migrations::new(next)).unwrap());
        assert_eq!(count(&backups), 1);

        // A failing migration aborts and leaves the database as it was.
        let mut bad: Vec<M> = schema::MIGRATIONS.iter().map(|sql| M::up(sql)).collect();
        bad.push(M::up("CREATE TABLE later (x INTEGER);"));
        bad.push(M::up("CREATE TABLE ok (x INTEGER); THIS IS NOT SQL;"));
        let err = Store::open_with(&db, &backups, Migrations::new(bad))
            .err()
            .unwrap();
        assert!(matches!(
            err,
            StoreError::Migration {
                backup: Some(_),
                ..
            }
        ));
        let c = Connection::open(&db).unwrap();
        let ok: Option<String> = c
            .query_row(
                "SELECT name FROM sqlite_master WHERE name = 'ok'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(ok, None);
    }
}
