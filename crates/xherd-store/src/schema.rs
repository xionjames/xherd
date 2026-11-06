//! Forward-only migrations. Never edit a released entry; append a new one.

pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema (design §3.3)
    r#"
CREATE TABLE agents      (id TEXT PRIMARY KEY, name TEXT UNIQUE NOT NULL, status TEXT NOT NULL, created_at INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE events      (id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT, at INTEGER NOT NULL);
CREATE INDEX events_agent ON events(agent_id, at);
CREATE INDEX events_at ON events(at);
CREATE TABLE notes       (id INTEGER PRIMARY KEY, agent_id TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL);
CREATE TABLE profiles    (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE todos       (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE approvals   (agent_id TEXT NOT NULL, file TEXT NOT NULL, sha256 TEXT NOT NULL, at INTEGER NOT NULL,
                          actor TEXT NOT NULL, auto INTEGER NOT NULL, reason TEXT, PRIMARY KEY (agent_id, file, sha256));
CREATE TABLE reviewed    (agent_id TEXT NOT NULL, file TEXT NOT NULL, text TEXT NOT NULL, PRIMARY KEY (agent_id, file));
CREATE TABLE diff_viewed (agent_id TEXT NOT NULL, path TEXT NOT NULL, fingerprint TEXT NOT NULL, PRIMARY KEY (agent_id, path));
CREATE TABLE tokens      (id TEXT PRIMARY KEY, kind TEXT NOT NULL, subject TEXT, scopes TEXT NOT NULL,
                          token_hash TEXT UNIQUE NOT NULL, created_at INTEGER NOT NULL, last_seen INTEGER, revoked_at INTEGER);
CREATE TABLE packs       (name TEXT PRIMARY KEY, version TEXT NOT NULL, source TEXT NOT NULL, sha TEXT, data TEXT NOT NULL);
CREATE TABLE installs    (runtime TEXT NOT NULL, pack TEXT NOT NULL, path TEXT NOT NULL, sha256 TEXT NOT NULL, PRIMARY KEY (runtime, pack, path));
CREATE TABLE plugins     (name TEXT PRIMARY KEY, kind TEXT NOT NULL, source TEXT NOT NULL, sha TEXT, enabled INTEGER NOT NULL, data TEXT NOT NULL);
CREATE TABLE worklog     (day TEXT NOT NULL, ticket_key TEXT NOT NULL, kind TEXT NOT NULL, ms INTEGER NOT NULL, PRIMARY KEY (day, ticket_key, kind));
CREATE TABLE budgets     (scope TEXT NOT NULL, day TEXT NOT NULL, tokens INTEGER NOT NULL, cost_micros INTEGER, PRIMARY KEY (scope, day));
CREATE TABLE memory_declined (repo TEXT NOT NULL, path TEXT NOT NULL, PRIMARY KEY (repo, path));
"#,
];
