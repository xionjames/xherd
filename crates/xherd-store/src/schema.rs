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
    // 2: an agent became a job (issue #141): `agents` -> `jobs`, `agent_id` -> `job_id`.
    // Rows, keys and indexes are kept (SQLite renames them in place).
    r#"
ALTER TABLE agents RENAME TO jobs;
ALTER TABLE events RENAME COLUMN agent_id TO job_id;
ALTER TABLE notes RENAME COLUMN agent_id TO job_id;
ALTER TABLE approvals RENAME COLUMN agent_id TO job_id;
ALTER TABLE reviewed RENAME COLUMN agent_id TO job_id;
ALTER TABLE diff_viewed RENAME COLUMN agent_id TO job_id;
DROP INDEX events_agent;
CREATE INDEX events_job ON events(job_id, at);
"#,
    // 3: saved agents (issue #141): the name `agents` is free after migration 2.
    // Every version, the current one included, is in `agent_versions`.
    r#"
CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT UNIQUE NOT NULL, archived_at INTEGER, data TEXT NOT NULL);
CREATE TABLE agent_versions (agent_id TEXT NOT NULL, version INTEGER NOT NULL, at INTEGER NOT NULL, data TEXT NOT NULL,
                             PRIMARY KEY (agent_id, version));
CREATE TABLE agent_memory (agent_id TEXT PRIMARY KEY, text TEXT NOT NULL, updated_at INTEGER NOT NULL, version INTEGER NOT NULL);
"#,
    // 4: an agent no longer names a workflow, repository, base branch or
    // stage overrides (issue #141, second part, D26). `AgentDef` refuses
    // unknown fields, so the keys go from the row and from every version.
    r#"
UPDATE agents SET data = json_remove(data, '$.workflow', '$.repo', '$.baseBranch', '$.stageOverrides');
UPDATE agent_versions SET data = json_remove(data, '$.workflow', '$.repo', '$.baseBranch', '$.stageOverrides');
"#,
    // 5: which saved agent runs which stage of a workflow (issue #141, D26).
    // One row per assigned stage; a stage with no row keeps the default
    // agent or the previous stage's.
    r#"
CREATE TABLE flow_assignments (pack TEXT NOT NULL, flow TEXT NOT NULL, stage TEXT NOT NULL, agent_id TEXT NOT NULL,
                               human INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (pack, flow, stage));
CREATE INDEX flow_assignments_agent ON flow_assignments(agent_id);
"#,
    // 6: the hash of the pack file version the user declined ("Keep mine"),
    // so an edited file is not offered again until the pack changes it.
    r#"
ALTER TABLE installs ADD COLUMN declined TEXT;
"#,
    // 7: the stages of a workflow that get a fresh-context check before
    // their gate opens. A row means on; no row means off.
    r#"
CREATE TABLE flow_fresh_checks (pack TEXT NOT NULL, flow TEXT NOT NULL, stage TEXT NOT NULL,
                                PRIMARY KEY (pack, flow, stage));
"#,
];
