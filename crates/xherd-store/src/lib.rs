//! SQLite store (design §3.3): WAL, one writer thread owning the only
//! read-write connection, a small pool of read-only connections, forward-only
//! migrations run at open after a `VACUUM INTO` copy (R-1305).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use serde_json::Value;
use xherd_core::{Agent, Millis};

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

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// An event in the agent log; `id` makes replays idempotent.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub payload: Option<Value>,
    pub at: Millis,
}

/// One row of the `approvals` table (design §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub agent_id: String,
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
    writer: std::sync::mpsc::Sender<Job>,
    readers: Arc<Mutex<Vec<Connection>>>,
    path: PathBuf,
}

const READERS: usize = 4;

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

        let (tx, rx) = std::sync::mpsc::channel::<Job>();
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

    pub async fn put_agent(&self, a: &Agent) -> Result<()> {
        let data = serde_json::to_string(a)?;
        let (id, name, status, created) = (
            a.id.clone(),
            a.name.clone(),
            a.status.to_string(),
            a.created_at,
        );
        self.write(move |c| {
            c.execute(
                "INSERT INTO agents (id, name, status, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, status = excluded.status, data = excluded.data",
                params![id, name, status, created, data],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        let id = id.to_string();
        self.read(move |c| {
            let data: Option<String> = c
                .query_row("SELECT data FROM agents WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
        })
        .await
    }

    pub async fn list_agents(&self) -> Result<Vec<Agent>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM agents ORDER BY created_at, id")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for d in rows {
                out.push(serde_json::from_str(&d?)?);
            }
            Ok(out)
        })
        .await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.write(move |c| {
            let tx = c.transaction()?;
            for t in ["events", "notes", "approvals", "reviewed", "diff_viewed"] {
                tx.execute(&format!("DELETE FROM {t} WHERE agent_id = ?1"), [&id])?;
            }
            tx.execute("DELETE FROM agents WHERE id = ?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    // ---- approvals -----------------------------------------------------------

    /// The ledger approves `file` at exactly this content (R-107).
    pub async fn approved(&self, agent_id: &str, file: &str, sha256: &str) -> Result<bool> {
        let (a, f, h) = (agent_id.to_string(), file.to_string(), sha256.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT 1 FROM approvals WHERE agent_id = ?1 AND file = ?2 AND sha256 = ?3",
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
                "INSERT OR IGNORE INTO approvals (agent_id, file, sha256, at, actor, auto, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    r.agent_id, r.file, r.sha256, r.at, r.actor, r.auto, r.reason
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Files with at least one approval for this agent.
    pub async fn approved_files(&self, agent_id: &str) -> Result<BTreeSet<String>> {
        let a = agent_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare("SELECT DISTINCT file FROM approvals WHERE agent_id = ?1")?;
            let rows = st.query_map([a], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<BTreeSet<String>>>()?)
        })
        .await
    }

    /// The text the user approved last, for "changed since you reviewed".
    pub async fn put_reviewed(&self, agent_id: &str, file: &str, text: &str) -> Result<()> {
        let (a, f, t) = (agent_id.to_string(), file.to_string(), text.to_string());
        self.write(move |c| {
            c.execute(
                "INSERT INTO reviewed (agent_id, file, text) VALUES (?1, ?2, ?3)
                 ON CONFLICT(agent_id, file) DO UPDATE SET text = excluded.text",
                params![a, f, t],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn reviewed(&self, agent_id: &str, file: &str) -> Result<Option<String>> {
        let (a, f) = (agent_id.to_string(), file.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT text FROM reviewed WHERE agent_id = ?1 AND file = ?2",
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
        agent_id: &str,
        path: &str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        let (a, p, f) = (
            agent_id.to_string(),
            path.to_string(),
            fingerprint.map(str::to_string),
        );
        self.write(move |c| {
            match f {
                Some(f) => c.execute(
                    "INSERT INTO diff_viewed (agent_id, path, fingerprint) VALUES (?1, ?2, ?3)
                     ON CONFLICT(agent_id, path) DO UPDATE SET fingerprint = excluded.fingerprint",
                    params![a, p, f],
                )?,
                None => c.execute(
                    "DELETE FROM diff_viewed WHERE agent_id = ?1 AND path = ?2",
                    params![a, p],
                )?,
            };
            Ok(())
        })
        .await
    }

    /// Path → fingerprint of every file the user marked viewed.
    pub async fn viewed(
        &self,
        agent_id: &str,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let a = agent_id.to_string();
        self.read(move |c| {
            let mut st =
                c.prepare("SELECT path, fingerprint FROM diff_viewed WHERE agent_id = ?1")?;
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
                "SELECT id, agent_id, kind, payload, at FROM events WHERE kind = ?1 ORDER BY at, id",
            )?;
            let rows = st.query_map([k], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?, r.get(4)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, agent_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event { id, agent_id, kind, payload, at });
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
                "INSERT OR IGNORE INTO events (id, agent_id, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.id, e.agent_id, e.kind, payload, e.at],
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

    /// `agent_id|at|gate` of every auto-approval marked seen (events of kind
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
    pub async fn events(&self, agent_id: &str, limit: usize) -> Result<Vec<Event>> {
        let id = agent_id.to_string();
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT id, agent_id, kind, payload, at FROM events WHERE agent_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![id, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?, r.get(4)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, agent_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event { id, agent_id, kind, payload, at });
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
                "SELECT id, agent_id, kind, payload, at FROM events
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
                let (id, agent_id, kind, payload, at) = row?;
                let payload = payload.map(|p| serde_json::from_str(&p)).transpose()?;
                out.push(Event {
                    id,
                    agent_id,
                    kind,
                    payload,
                    at,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Delete events older than `before`. Returns how many were removed.
    pub async fn prune_events(&self, before: Millis) -> Result<usize> {
        self.write(move |c| Ok(c.execute("DELETE FROM events WHERE at < ?1", [before])?))
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

    #[tokio::test]
    async fn agent_roundtrip_and_update() {
        let (_d, s) = open();
        let mut a = Agent::new("SC-1", "SC-1", 1);
        s.put_agent(&a).await.unwrap();
        a.status = Status::Running;
        s.put_agent(&a).await.unwrap();
        assert_eq!(s.get_agent(&a.id).await.unwrap(), Some(a.clone()));
        assert_eq!(s.list_agents().await.unwrap().len(), 1);
        s.delete_agent(&a.id).await.unwrap();
        assert_eq!(s.get_agent(&a.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn events_are_idempotent_and_pruned() {
        let (_d, s) = open();
        let e = Event {
            id: "e1".into(),
            agent_id: "a".into(),
            kind: "report.stop".into(),
            payload: None,
            at: 10,
        };
        assert!(s.insert_event(e.clone()).await.unwrap());
        assert!(!s.insert_event(e).await.unwrap());
        assert!(s.event_exists("e1").await.unwrap());
        s.insert_event(Event {
            id: "e2".into(),
            agent_id: "a".into(),
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
