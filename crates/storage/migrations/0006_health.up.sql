CREATE TABLE health_tasks (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 request_id TEXT NOT NULL UNIQUE,
 trigger TEXT NOT NULL CHECK(trigger IN ('manual','automatic')),
 created_at INTEGER NOT NULL,
 merged INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE health_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 node_id TEXT NOT NULL,
 identity TEXT NOT NULL,
 profile_fingerprint TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('queued','probing','completed','interrupted','discarded')),
 created_at INTEGER NOT NULL,
 started_at INTEGER,
 finished_at INTEGER,
 result TEXT
);
CREATE UNIQUE INDEX health_one_inflight ON health_runs(identity,profile_fingerprint) WHERE state IN ('queued','probing');
CREATE INDEX health_pending ON health_runs(state,id);
CREATE TABLE health_task_runs (
 task_id INTEGER NOT NULL REFERENCES health_tasks(id) ON DELETE CASCADE,
 run_id INTEGER NOT NULL REFERENCES health_runs(id) ON DELETE CASCADE,
 PRIMARY KEY(task_id,run_id)
);
CREATE INDEX health_task_run ON health_task_runs(run_id);
CREATE TABLE health_state (
 identity TEXT NOT NULL,
 profile_fingerprint TEXT NOT NULL,
 result TEXT NOT NULL,
 checked_at INTEGER NOT NULL,
 failures INTEGER NOT NULL DEFAULT 0,
 next_due_at INTEGER NOT NULL,
 PRIMARY KEY(identity,profile_fingerprint)
);
CREATE TABLE health_daily_usage(day INTEGER PRIMARY KEY, probes INTEGER NOT NULL CHECK(probes>=0));
