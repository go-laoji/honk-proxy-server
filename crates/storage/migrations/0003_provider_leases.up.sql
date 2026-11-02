CREATE TABLE provider_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 64),
    request_id TEXT NOT NULL CHECK(length(request_id) BETWEEN 1 AND 64),
    fingerprint TEXT NOT NULL CHECK(length(fingerprint)=64),
    state TEXT NOT NULL CHECK(state IN ('accepted','requesting','succeeded','failed','outcome_unknown','discarded')),
    requested_count INTEGER NOT NULL CHECK(requested_count BETWEEN 1 AND 1000),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    added INTEGER NOT NULL DEFAULT 0,
    rejected INTEGER NOT NULL DEFAULT 0,
    removed INTEGER NOT NULL DEFAULT 0,
    quota INTEGER,
    error_code TEXT,
    retry_after_secs INTEGER,
    UNIQUE(provider_id,request_id)
);
CREATE UNIQUE INDEX provider_tasks_inflight ON provider_tasks(provider_id) WHERE state IN ('accepted','requesting');
CREATE INDEX provider_tasks_history ON provider_tasks(provider_id,id DESC);
CREATE TABLE provider_batches (
    task_id INTEGER PRIMARY KEY REFERENCES provider_tasks(id) ON DELETE RESTRICT,
    provider_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    mode TEXT NOT NULL CHECK(mode IN ('append_leases','replace_snapshot'))
);
CREATE TABLE provider_leases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES provider_batches(task_id) ON DELETE RESTRICT,
    provider_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    document TEXT NOT NULL CHECK(length(document)<=16384),
    received_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active','revoked','clock_rollback')),
    CHECK(expires_at>received_at)
);
CREATE INDEX provider_leases_pool ON provider_leases(state,expires_at);
CREATE INDEX provider_leases_source ON provider_leases(provider_id,id DESC);
CREATE TABLE provider_clock (singleton INTEGER PRIMARY KEY CHECK(singleton=1), observed_at INTEGER NOT NULL);
