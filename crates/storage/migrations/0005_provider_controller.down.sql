DROP TABLE provider_control;
DROP TABLE provider_daily_usage;
ALTER TABLE provider_tasks DROP COLUMN min_interval_secs;
ALTER TABLE provider_tasks DROP COLUMN policy_json;
ALTER TABLE provider_tasks DROP COLUMN trigger;
