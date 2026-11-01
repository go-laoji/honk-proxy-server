-- Persistent projections are written by the same transaction as their source facts.
CREATE TABLE task_records (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 source_ref TEXT UNIQUE,
 kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 32),
 trigger TEXT NOT NULL DEFAULT 'manual',
 version INTEGER NOT NULL DEFAULT 1 CHECK(version>0),
 state TEXT NOT NULL CHECK(state IN ('queued','running','completed','failed','outcome_unknown','discarded','interrupted','pending_restart')),
 phase TEXT NOT NULL CHECK(length(phase) BETWEEN 1 AND 64),
 terminal INTEGER NOT NULL CHECK(terminal IN (0,1)),
 created_at INTEGER NOT NULL CHECK(created_at>=0), started_at INTEGER,
 updated_at INTEGER NOT NULL CHECK(updated_at>=created_at), finished_at INTEGER,
 configuration_revision INTEGER, configuration_fingerprint TEXT,
 resource_kind TEXT, resource_id TEXT, request_id TEXT,
 scope TEXT NOT NULL DEFAULT '', input_fingerprint TEXT,
 total INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0,
 succeeded INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0,
 skipped INTEGER NOT NULL DEFAULT 0, removed INTEGER NOT NULL DEFAULT 0,
 error_code TEXT, result_ref TEXT,
 result_json TEXT CHECK(result_json IS NULL OR (json_valid(result_json) AND length(result_json)<=65536))
);
CREATE INDEX task_records_history ON task_records(kind,state,id DESC);
CREATE INDEX task_records_terminal ON task_records(terminal,id DESC);
CREATE INDEX task_records_request ON task_records(request_id,id DESC);
CREATE UNIQUE INDEX task_work_idempotency ON task_records(request_id) WHERE source_ref IS NULL AND request_id IS NOT NULL;
CREATE TABLE event_context (singleton INTEGER PRIMARY KEY CHECK(singleton=1), epoch TEXT NOT NULL);
INSERT INTO event_context VALUES(1,'00000000-0000-0000-0000-000000000000');
CREATE TABLE service_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 recorded_epoch TEXT NOT NULL,
 occurred_at INTEGER NOT NULL,
 level TEXT NOT NULL CHECK(level IN ('info','warn','error')),
 module TEXT NOT NULL CHECK(length(module) BETWEEN 1 AND 32),
 code TEXT NOT NULL CHECK(length(code) BETWEEN 1 AND 96),
 task_id INTEGER, task_version INTEGER, request_id TEXT,
 resource_kind TEXT, resource_id TEXT, node_id TEXT, provider_id TEXT,
 peer TEXT, action TEXT, outcome TEXT, configuration_revision INTEGER
);
CREATE INDEX service_events_module ON service_events(module,sequence);
CREATE INDEX service_events_task ON service_events(task_id,sequence);
CREATE INDEX service_events_request ON service_events(request_id,sequence);
CREATE INDEX service_events_node ON service_events(node_id,sequence);
CREATE INDEX service_events_provider ON service_events(provider_id,sequence);
CREATE INDEX service_events_age ON service_events(occurred_at);

CREATE VIEW hps_provider_task_projection AS SELECT
 id AS source_id, 'provider:'||id AS source_ref, 'provider' AS kind, trigger,
 CASE state WHEN 'accepted' THEN 'queued' WHEN 'requesting' THEN 'running' WHEN 'succeeded' THEN 'completed' ELSE state END AS state,
 state AS phase, state NOT IN ('accepted','requesting') AS terminal,
 started_at*1000 AS created_at, started_at*1000 AS started_at,
 max(started_at,coalesce(finished_at,started_at))*1000 AS updated_at,
 finished_at*1000 AS finished_at, NULL AS configuration_revision,
 fingerprint AS configuration_fingerprint, 'provider' AS resource_kind,
 provider_id AS resource_id, request_id, requested_count AS total,
 added+rejected AS completed, added AS succeeded, rejected AS failed, 0 AS skipped,
 removed, error_code, 'provider:'||id AS result_ref FROM provider_tasks;

CREATE VIEW hps_activation_task_projection AS SELECT
 id AS source_id, 'activation:'||id AS source_ref, 'activation' AS kind, 'configuration' AS trigger,
 CASE state WHEN 'prepared' THEN 'running' WHEN 'applied' THEN 'completed' ELSE state END AS state,
 state AS phase, state NOT IN ('prepared','pending_restart') AS terminal,
 created_at*1000 AS created_at, created_at*1000 AS started_at,
 created_at*1000 AS updated_at,
 CASE WHEN state='failed' THEN created_at*1000 ELSE NULL END AS finished_at,
 revision AS configuration_revision, NULL AS configuration_fingerprint,
 'configuration' AS resource_kind, cast(revision AS TEXT) AS resource_id, NULL AS request_id,
 1 AS total, state NOT IN ('prepared','pending_restart') AS completed,
 state='applied' AS succeeded, state='failed' AS failed, 0 AS skipped, 0 AS removed,
 error_code, 'activation:'||id AS result_ref FROM config_activations;

CREATE VIEW hps_health_task_projection AS SELECT
 h.id AS source_id, 'health:'||h.id AS source_ref, 'health' AS kind, h.trigger,
 CASE WHEN coalesce(a.inflight,0)>0 THEN 'running' WHEN coalesce(a.interrupted,0)>0 THEN 'interrupted' ELSE 'completed' END AS state,
 CASE WHEN coalesce(a.probing,0)>0 THEN 'probing' WHEN coalesce(a.inflight,0)>0 THEN 'queued' WHEN coalesce(a.interrupted,0)>0 THEN 'interrupted' ELSE 'completed' END AS phase,
 coalesce(a.inflight,0)=0 AS terminal,
 h.created_at*1000 AS created_at, a.first_started*1000 AS started_at,
 max(h.created_at,coalesce(a.last_updated,h.created_at))*1000 AS updated_at,
 CASE WHEN coalesce(a.inflight,0)=0 THEN max(h.created_at,coalesce(a.last_updated,h.created_at))*1000 ELSE NULL END AS finished_at,
 NULL AS configuration_revision, NULL AS configuration_fingerprint,
 'health' AS resource_kind, NULL AS resource_id, h.request_id,
 coalesce(a.total,0) AS total, coalesce(a.completed,0) AS completed,
 coalesce(a.healthy,0) AS succeeded,
 coalesce(a.completed,0)-coalesce(a.healthy,0)-coalesce(a.skipped,0) AS failed,
 coalesce(a.skipped,0) AS skipped, 0 AS removed,
 CASE WHEN coalesce(a.interrupted,0)>0 THEN 'process_interrupted' ELSE NULL END AS error_code,
 'health:'||h.id AS result_ref
 FROM health_tasks h LEFT JOIN (
 SELECT l.task_id, count(*) AS total,
 sum(r.state NOT IN ('queued','probing')) AS completed,
 sum(coalesce(json_extract(r.result,'$.verdict')='healthy',0)) AS healthy,
 sum(r.state='discarded' OR coalesce(json_extract(r.result,'$.verdict') IN ('skipped','expired','unsupported'),0)) AS skipped,
 sum(r.state IN ('queued','probing')) AS inflight,
 sum(r.state='probing') AS probing, sum(r.state='interrupted') AS interrupted,
 min(r.started_at) AS first_started,
 max(coalesce(r.finished_at,r.started_at,r.created_at)) AS last_updated
 FROM health_task_runs l JOIN health_runs r ON r.id=l.run_id GROUP BY l.task_id
 ) a ON a.task_id=h.id;
INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_provider_task_projection ORDER BY source_id;
INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_activation_task_projection ORDER BY source_id;
INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_health_task_projection ORDER BY source_id;
DELETE FROM task_records WHERE terminal=1 AND id < coalesce((SELECT id FROM task_records WHERE terminal=1 ORDER BY id DESC LIMIT 1 OFFSET 9999),0);

CREATE TRIGGER task_event_insert AFTER INSERT ON task_records BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,task_id,task_version,request_id,resource_kind,resource_id,provider_id,configuration_revision)
 VALUES((SELECT epoch FROM event_context),NEW.updated_at,
 CASE WHEN NEW.state='failed' THEN 'error' WHEN NEW.state IN ('interrupted','outcome_unknown') THEN 'warn' ELSE 'info' END,
 CASE WHEN NEW.kind IN ('provider','health') THEN NEW.kind WHEN NEW.kind IN ('activation','configuration_prepare','configuration_validate') THEN 'configuration' ELSE 'diagnostic' END,
 'task_'||NEW.state,NEW.id,NEW.version,NEW.request_id,NEW.resource_kind,NEW.resource_id,
 CASE WHEN NEW.resource_kind='provider' THEN NEW.resource_id ELSE NULL END,NEW.configuration_revision);
END;
CREATE TRIGGER task_event_update AFTER UPDATE ON task_records WHEN NEW.version>OLD.version BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,task_id,task_version,request_id,resource_kind,resource_id,provider_id,configuration_revision)
 VALUES((SELECT epoch FROM event_context),NEW.updated_at,
 CASE WHEN NEW.state='failed' THEN 'error' WHEN NEW.state IN ('interrupted','outcome_unknown') THEN 'warn' ELSE 'info' END,
 CASE WHEN NEW.kind IN ('provider','health') THEN NEW.kind WHEN NEW.kind IN ('activation','configuration_prepare','configuration_validate') THEN 'configuration' ELSE 'diagnostic' END,
 'task_'||NEW.state,NEW.id,NEW.version,NEW.request_id,NEW.resource_kind,NEW.resource_id,
 CASE WHEN NEW.resource_kind='provider' THEN NEW.resource_id ELSE NULL END,NEW.configuration_revision);
END;
CREATE TRIGGER service_events_bound AFTER INSERT ON service_events BEGIN
 DELETE FROM service_events WHERE sequence<=NEW.sequence-10000;
END;

CREATE TRIGGER provider_task_insert AFTER INSERT ON provider_tasks BEGIN
 INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_provider_task_projection WHERE source_id=NEW.id ON CONFLICT(source_ref) DO UPDATE SET version=task_records.version+1,state=excluded.state,phase=excluded.phase,terminal=excluded.terminal,started_at=excluded.started_at,updated_at=max(task_records.updated_at,excluded.updated_at,unixepoch()*1000),finished_at=excluded.finished_at,configuration_revision=coalesce(task_records.configuration_revision,excluded.configuration_revision),configuration_fingerprint=excluded.configuration_fingerprint,resource_kind=excluded.resource_kind,resource_id=excluded.resource_id,request_id=excluded.request_id,total=excluded.total,completed=excluded.completed,succeeded=excluded.succeeded,failed=excluded.failed,skipped=excluded.skipped,removed=excluded.removed,error_code=excluded.error_code,result_ref=excluded.result_ref;
END;

CREATE TRIGGER provider_task_update AFTER UPDATE ON provider_tasks BEGIN
 INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_provider_task_projection WHERE source_id=NEW.id ON CONFLICT(source_ref) DO UPDATE SET version=task_records.version+1,state=excluded.state,phase=excluded.phase,terminal=excluded.terminal,started_at=excluded.started_at,updated_at=max(task_records.updated_at,excluded.updated_at,unixepoch()*1000),finished_at=excluded.finished_at,configuration_revision=coalesce(task_records.configuration_revision,excluded.configuration_revision),configuration_fingerprint=excluded.configuration_fingerprint,resource_kind=excluded.resource_kind,resource_id=excluded.resource_id,request_id=excluded.request_id,total=excluded.total,completed=excluded.completed,succeeded=excluded.succeeded,failed=excluded.failed,skipped=excluded.skipped,removed=excluded.removed,error_code=excluded.error_code,result_ref=excluded.result_ref;
END;

CREATE TRIGGER activation_task_insert AFTER INSERT ON config_activations BEGIN
 INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_activation_task_projection WHERE source_id=NEW.id ON CONFLICT(source_ref) DO UPDATE SET version=task_records.version+1,state=excluded.state,phase=excluded.phase,terminal=excluded.terminal,started_at=excluded.started_at,updated_at=max(task_records.updated_at,excluded.updated_at,unixepoch()*1000),finished_at=excluded.finished_at,configuration_revision=coalesce(task_records.configuration_revision,excluded.configuration_revision),configuration_fingerprint=excluded.configuration_fingerprint,resource_kind=excluded.resource_kind,resource_id=excluded.resource_id,request_id=excluded.request_id,total=excluded.total,completed=excluded.completed,succeeded=excluded.succeeded,failed=excluded.failed,skipped=excluded.skipped,removed=excluded.removed,error_code=excluded.error_code,result_ref=excluded.result_ref;
END;

CREATE TRIGGER activation_task_update AFTER UPDATE ON config_activations BEGIN
 INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_activation_task_projection WHERE source_id=NEW.id ON CONFLICT(source_ref) DO UPDATE SET version=task_records.version+1,state=excluded.state,phase=excluded.phase,terminal=excluded.terminal,started_at=excluded.started_at,updated_at=max(task_records.updated_at,excluded.updated_at,unixepoch()*1000),finished_at=CASE WHEN excluded.terminal=1 THEN coalesce(task_records.finished_at,unixepoch()*1000) ELSE NULL END,configuration_revision=coalesce(task_records.configuration_revision,excluded.configuration_revision),configuration_fingerprint=excluded.configuration_fingerprint,resource_kind=excluded.resource_kind,resource_id=excluded.resource_id,request_id=excluded.request_id,total=excluded.total,completed=excluded.completed,succeeded=excluded.succeeded,failed=excluded.failed,skipped=excluded.skipped,removed=excluded.removed,error_code=excluded.error_code,result_ref=excluded.result_ref;
END;

CREATE TRIGGER health_run_task_update AFTER UPDATE ON health_runs WHEN NEW.state<>OLD.state BEGIN
 INSERT INTO task_records(source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref) SELECT source_ref,kind,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref FROM hps_health_task_projection WHERE source_id IN (SELECT task_id FROM health_task_runs WHERE run_id=NEW.id) ON CONFLICT(source_ref) DO UPDATE SET version=task_records.version+1,state=excluded.state,phase=excluded.phase,terminal=excluded.terminal,started_at=excluded.started_at,updated_at=max(task_records.updated_at,excluded.updated_at,unixepoch()*1000),finished_at=excluded.finished_at,configuration_revision=coalesce(task_records.configuration_revision,excluded.configuration_revision),configuration_fingerprint=excluded.configuration_fingerprint,resource_kind=excluded.resource_kind,resource_id=excluded.resource_id,request_id=excluded.request_id,total=excluded.total,completed=excluded.completed,succeeded=excluded.succeeded,failed=excluded.failed,skipped=excluded.skipped,removed=excluded.removed,error_code=excluded.error_code,result_ref=excluded.result_ref;
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,task_id,task_version,request_id,resource_kind,resource_id,node_id,configuration_revision)
 SELECT (SELECT epoch FROM event_context),max(NEW.created_at,coalesce(NEW.finished_at,NEW.started_at))*1000,
 CASE WHEN NEW.state='interrupted' THEN 'warn' ELSE 'info' END,'health','probe_'||NEW.state,
 t.id,t.version,t.request_id,'node',NEW.node_id,NEW.node_id,t.configuration_revision
 FROM task_records t WHERE t.source_ref IN (SELECT 'health:'||task_id FROM health_task_runs WHERE run_id=NEW.id);
END;

CREATE TRIGGER task_terminal_bound AFTER UPDATE ON task_records WHEN NEW.terminal=1 AND OLD.terminal=0 BEGIN
 DELETE FROM task_records WHERE terminal=1 AND id<>NEW.id AND id < coalesce((SELECT id FROM task_records WHERE terminal=1 AND id<>NEW.id ORDER BY id DESC LIMIT 1 OFFSET 9998),0);
END;

CREATE TRIGGER task_terminal_insert_bound AFTER INSERT ON task_records WHEN NEW.terminal=1 BEGIN
 DELETE FROM task_records WHERE terminal=1 AND id<>NEW.id AND id < coalesce((SELECT id FROM task_records WHERE terminal=1 AND id<>NEW.id ORDER BY id DESC LIMIT 1 OFFSET 9998),0);
END;

CREATE TRIGGER node_control_event_insert AFTER INSERT ON node_controls BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,resource_kind,resource_id,node_id,action,outcome)
 VALUES((SELECT epoch FROM event_context),NEW.updated_at*1000,'info','runtime','node_'||NEW.state,'node',NEW.node_id,NEW.node_id,'node_control','succeeded');
END;

CREATE TRIGGER node_control_event_update AFTER UPDATE ON node_controls BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,resource_kind,resource_id,node_id,action,outcome)
 VALUES((SELECT epoch FROM event_context),NEW.updated_at*1000,'info','runtime','node_'||NEW.state,'node',NEW.node_id,NEW.node_id,'node_control','succeeded');
END;

CREATE TRIGGER configuration_revision_event AFTER INSERT ON config_revisions WHEN NEW.parent_id IS NOT NULL BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,resource_kind,resource_id,configuration_revision,action,outcome)
 VALUES((SELECT epoch FROM event_context),NEW.created_at*1000,'info','configuration','configuration_revision_saved','configuration',cast(NEW.id AS TEXT),NEW.id,'configuration_prepare','succeeded');
END;
CREATE TRIGGER configuration_draft_event AFTER UPDATE ON config_draft BEGIN
 INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,resource_kind,resource_id,configuration_revision,action,outcome)
 VALUES((SELECT epoch FROM event_context),NEW.updated_at*1000,'info','configuration','configuration_draft_saved','configuration',cast(NEW.base_revision AS TEXT),NEW.base_revision,'configuration_draft','succeeded');
END;
