ALTER TABLE provider_tasks ADD COLUMN trigger TEXT NOT NULL DEFAULT 'manual' CHECK(trigger IN ('manual','automatic'));
ALTER TABLE provider_tasks ADD COLUMN policy_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE provider_tasks ADD COLUMN min_interval_secs INTEGER NOT NULL DEFAULT 60;
CREATE TABLE provider_daily_usage (
    provider_id TEXT NOT NULL,
    day INTEGER NOT NULL,
    requests INTEGER NOT NULL CHECK(requests>=0),
    reserved_nodes INTEGER NOT NULL CHECK(reserved_nodes>=0),
    PRIMARY KEY(provider_id,day)
);
INSERT INTO provider_daily_usage(provider_id,day,requests,reserved_nodes)
SELECT provider_id,started_at/86400,count(*),sum(requested_count) FROM provider_tasks GROUP BY provider_id,started_at/86400;
CREATE TABLE provider_control (
    provider_id TEXT PRIMARY KEY,
    refilling INTEGER NOT NULL DEFAULT 0 CHECK(refilling IN (0,1)),
    next_allowed_at INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0 CHECK(failure_count>=0),
    paused_reason TEXT,
    last_task_id INTEGER,
    acknowledged_at INTEGER,
    updated_at INTEGER NOT NULL
);
INSERT INTO provider_control(provider_id,next_allowed_at,paused_reason,last_task_id,updated_at)
SELECT provider_id,started_at+60,CASE WHEN state='outcome_unknown' THEN 'outcome_unknown' END,id,started_at
FROM provider_tasks WHERE id IN (SELECT max(id) FROM provider_tasks GROUP BY provider_id);
