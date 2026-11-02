//! Bounded durable task projections and structured event cursors on the SQLite owner.
use super::*;
use hps_domain::{
    EVENT_MODULES, EventQuery, EventsOverview, ServiceEvent, TASK_KINDS, TASK_STATES, TaskInfo,
    TaskQuery, TasksOverview, event_cursor, parse_event_cursor,
};

const TASK_COLUMNS: &str = "id,version,kind,source_ref,trigger,state,phase,terminal,created_at,started_at,updated_at,finished_at,configuration_revision,configuration_fingerprint,resource_kind,resource_id,request_id,total,completed,succeeded,failed,skipped,removed,error_code,result_ref";
const EVENT_COLUMNS: &str = "sequence,recorded_epoch,occurred_at,level,module,code,task_id,task_version,request_id,resource_kind,resource_id,node_id,provider_id,peer,action,outcome,configuration_revision";
pub const TASK_RETENTION: u32 = 10_000;
pub const EVENT_RETENTION: u32 = 10_000;

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64
}
fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    value
        .try_into()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn optional_unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(index)?;
    value
        .map(|n| {
            n.try_into()
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, n))
        })
        .transpose()
}
fn task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskInfo> {
    let result: Option<String> = row.get(25)?;
    Ok(TaskInfo {
        id: row.get(0)?,
        version: unsigned(row, 1)?,
        kind: row.get(2)?,
        source_ref: row.get(3)?,
        trigger: row.get(4)?,
        state: row.get(5)?,
        phase: row.get(6)?,
        terminal: row.get(7)?,
        created_at_unix_ms: unsigned(row, 8)?,
        started_at_unix_ms: optional_unsigned(row, 9)?,
        updated_at_unix_ms: unsigned(row, 10)?,
        finished_at_unix_ms: optional_unsigned(row, 11)?,
        configuration_revision: row.get(12)?,
        configuration_fingerprint: row.get(13)?,
        resource_kind: row.get(14)?,
        resource_id: row.get(15)?,
        request_id: row.get(16)?,
        total: unsigned(row, 17)?,
        completed: unsigned(row, 18)?,
        succeeded: unsigned(row, 19)?,
        failed: unsigned(row, 20)?,
        skipped: unsigned(row, 21)?,
        removed: unsigned(row, 22)?,
        error_code: row.get(23)?,
        result_ref: row.get(24)?,
        result: result
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    25,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
    })
}
fn read_task(c: &Connection, id: i64) -> Result<TaskInfo, StorageError> {
    c.query_row(
        &format!("SELECT {TASK_COLUMNS},result_json FROM task_records WHERE id=?1"),
        [id],
        task_row,
    )
    .optional()?
    .ok_or(StorageError::NotFound)
}
fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServiceEvent> {
    Ok(ServiceEvent {
        sequence: row.get(0)?,
        recorded_epoch: row.get(1)?,
        occurred_at_unix_ms: unsigned(row, 2)?,
        level: row.get(3)?,
        module: row.get(4)?,
        code: row.get(5)?,
        task_id: row.get(6)?,
        task_version: optional_unsigned(row, 7)?,
        request_id: row.get(8)?,
        resource_kind: row.get(9)?,
        resource_id: row.get(10)?,
        node_id: row.get(11)?,
        provider_id: row.get(12)?,
        peer: row.get(13)?,
        action: row.get(14)?,
        outcome: row.get(15)?,
        configuration_revision: row.get(16)?,
    })
}
fn identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn text(value: &Option<String>, maximum: usize) -> bool {
    value.as_ref().is_none_or(|s| identifier(s, maximum))
}
fn valid_task_query(q: &TaskQuery) -> bool {
    q.source_ref
        .as_ref()
        .is_none_or(|s| hps_domain::valid_task_source_ref(s))
        && q.before.is_none_or(|n| n > 0)
        && q.limit.is_none_or(|n| (1..=200).contains(&n))
        && q.kind
            .as_ref()
            .is_none_or(|s| TASK_KINDS.contains(&s.as_str()))
        && q.state
            .as_ref()
            .is_none_or(|s| TASK_STATES.contains(&s.as_str()))
        && text(&q.resource_kind, 32)
        && text(&q.resource_id, 128)
        && text(&q.request_id, 64)
        && q.created_after_unix_ms.is_none_or(|n| n <= i64::MAX as u64)
}

/// Only safe metadata is accepted. Input bodies and credentials are never retained here.
#[derive(Clone)]
pub struct WorkInput {
    pub kind: &'static str,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub request_id: String,
    pub input_fingerprint: String,
    pub configuration_revision: Option<i64>,
}
fn validate_work(input: &WorkInput) -> Result<(), StorageError> {
    if !TASK_KINDS.contains(&input.kind)
        || matches!(input.kind, "provider" | "health" | "activation")
        || !text(&input.resource_kind, 32)
        || !text(&input.resource_id, 128)
        || !identifier(&input.request_id, 64)
        || input.input_fingerprint.len() != 64
        || !input
            .input_fingerprint
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || input.configuration_revision.is_some_and(|n| n <= 0)
    {
        return Err(StorageError::Invalid);
    }
    Ok(())
}
fn work_scope(input: &WorkInput) -> String {
    if input.resource_kind.as_deref() == Some("configuration") {
        "configuration".to_owned()
    } else {
        format!(
            "{}:{}",
            input.resource_kind.as_deref().unwrap_or(""),
            input.resource_id.as_deref().unwrap_or("")
        )
    }
}
fn existing_work(c: &Connection, input: &WorkInput) -> Result<Option<TaskInfo>, StorageError> {
    let record=c.query_row("SELECT id,input_fingerprint,kind,scope FROM task_records WHERE source_ref IS NULL AND request_id=?1",[&input.request_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
    record
        .map(|(id, fp, kind, scope)| {
            if fp != input.input_fingerprint || kind != input.kind || scope != work_scope(input) {
                return Err(StorageError::Conflict);
            }
            read_task(c, id)
        })
        .transpose()
}
pub struct WorkResult {
    pub state: &'static str,
    pub error_code: Option<&'static str>,
    pub result_ref: Option<String>,
    pub result: Option<serde_json::Value>,
}
pub struct ManagementEvent {
    pub request_id: String,
    pub peer: Option<std::net::SocketAddr>,
    pub action: &'static str,
    pub outcome: &'static str,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: Option<String>,
    pub configuration_revision: Option<i64>,
}

impl Store {
    pub async fn record_management_event(
        &self,
        input: ManagementEvent,
    ) -> Result<(), StorageError> {
        if !identifier(&input.request_id, 64)
            || !identifier(input.action, 64)
            || !["started", "accepted", "succeeded", "rejected", "failed"].contains(&input.outcome)
            || !text(&input.resource_kind, 32)
            || !text(&input.resource_id, 128)
            || !text(&input.node_id, 128)
            || !text(&input.provider_id, 64)
            || input.configuration_revision.is_some_and(|n| n <= 0)
        {
            return Err(StorageError::Invalid);
        }
        self.call(move|c|{
            c.execute("INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code,request_id,resource_kind,resource_id,node_id,provider_id,peer,action,outcome,configuration_revision) VALUES((SELECT epoch FROM event_context),?1,?2,'management','management_operation',?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![unix_ms() as i64,if input.outcome=="failed" {"error"} else if input.outcome=="rejected" {"warn"} else {"info"},input.request_id,input.resource_kind,input.resource_id,input.node_id,input.provider_id,input.peer.map(|p|p.to_string()),input.action,input.outcome,input.configuration_revision])?;Ok(())
        }).await
    }
    pub async fn task(&self, id: i64) -> Result<TaskInfo, StorageError> {
        if id <= 0 {
            return Err(StorageError::Invalid);
        }
        self.call(move |c| read_task(c, id)).await
    }
    pub async fn tasks(&self, query: TaskQuery) -> Result<TasksOverview, StorageError> {
        if !valid_task_query(&query) {
            return Err(StorageError::Invalid);
        }
        if query
            .expected_epoch
            .as_ref()
            .is_some_and(|e| e != &self.epoch)
        {
            return Err(StorageError::Conflict);
        }
        let epoch = self.epoch.clone();
        self.call(move |c| {
            let limit=query.limit.unwrap_or(50);
            let mut records=c.prepare(&format!("SELECT {TASK_COLUMNS},NULL FROM task_records WHERE (?1 IS NULL OR id<?1) AND (?2 IS NULL OR kind=?2) AND (?3 IS NULL OR state=?3) AND (?4 IS NULL OR resource_kind=?4) AND (?5 IS NULL OR resource_id=?5) AND (?6 IS NULL OR request_id=?6) AND (?7 IS NULL OR created_at>=?7) AND (?8 IS NULL OR source_ref=?8) ORDER BY id DESC LIMIT ?9"))?
                .query_map(params![query.before,query.kind,query.state,query.resource_kind,query.resource_id,query.request_id,query.created_after_unix_ms.map(|n|n as i64),query.source_ref,limit+1],task_row)?.collect::<Result<Vec<_>,_>>()?;
            let more=records.len()>limit as usize;
            records.truncate(limit as usize);
            Ok(TasksOverview {epoch, observed_at_unix_ms:unix_ms(),next_before:if more {records.last().map(|t|t.id)} else {None},records,terminal_retention_limit:TASK_RETENTION})
        }).await
    }
    pub async fn existing_work(&self, input: WorkInput) -> Result<Option<TaskInfo>, StorageError> {
        validate_work(&input)?;
        self.call(move |c| existing_work(c, &input)).await
    }
    pub async fn begin_work(&self, input: WorkInput) -> Result<(TaskInfo, bool), StorageError> {
        validate_work(&input)?;
        self.call(move |c| {
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let scope=work_scope(&input);
            if let Some(task)=existing_work(&tx,&input)? { return Ok((task,false)); }
            let active:u32=tx.query_row("SELECT count(*) FROM task_records WHERE source_ref IS NULL AND terminal=0",[],|r|r.get(0))?;
            if active>=64 {return Err(StorageError::TaskCapacity);}
            let now=unix_ms() as i64;
            tx.execute("INSERT INTO task_records(kind,trigger,state,phase,terminal,created_at,updated_at,configuration_revision,resource_kind,resource_id,request_id,scope,input_fingerprint,total) VALUES(?1,'manual','queued','queued',0,?2,?2,?3,?4,?5,?6,?7,?8,1)",params![input.kind,now,input.configuration_revision,input.resource_kind,input.resource_id,input.request_id,scope,input.input_fingerprint])?;
            let result=read_task(&tx,tx.last_insert_rowid())?;tx.commit()?;Ok((result,true))
        }).await
    }
    pub async fn start_work(&self, id: i64) -> Result<TaskInfo, StorageError> {
        self.call(move |c| {
            if c.execute("UPDATE task_records SET version=version+1,state='running',phase='running',started_at=?2,updated_at=max(updated_at,?2) WHERE id=?1 AND source_ref IS NULL AND state='queued'",params![id,unix_ms() as i64])?!=1 {return Err(StorageError::Conflict);}
            read_task(c,id)
        }).await
    }
    pub async fn finish_work(&self, id: i64, result: WorkResult) -> Result<TaskInfo, StorageError> {
        if ![
            "completed",
            "failed",
            "interrupted",
            "outcome_unknown",
            "discarded",
        ]
        .contains(&result.state)
            || result.error_code.is_some_and(|s| !identifier(s, 96))
            || !text(&result.result_ref, 128)
        {
            return Err(StorageError::Invalid);
        }
        let document = result
            .result
            .map(|v| serde_json::to_string(&v))
            .transpose()
            .map_err(|_| StorageError::Invalid)?;
        if document.as_ref().is_some_and(|s| s.len() > 65536) {
            return Err(StorageError::Invalid);
        }
        self.call(move |c| {
            if c.execute("UPDATE task_records SET version=version+1,state=?2,phase=?2,terminal=1,updated_at=max(updated_at,?3),finished_at=?3,completed=1,succeeded=?4,failed=?5,error_code=?6,result_ref=?7,result_json=?8 WHERE id=?1 AND source_ref IS NULL AND state IN ('queued','running')",params![id,result.state,unix_ms() as i64,u32::from(result.state=="completed"),u32::from(result.state=="failed"),result.error_code,result.result_ref,document])?!=1 {return Err(StorageError::Conflict);}
            read_task(c,id)
        }).await
    }
    pub async fn annotate_task_source(
        &self,
        source_ref: String,
        revision: i64,
    ) -> Result<(), StorageError> {
        if !identifier(&source_ref, 128) || revision <= 0 {
            return Err(StorageError::Invalid);
        }
        self.call(move|c|{c.execute("UPDATE task_records SET version=version+1,configuration_revision=?2 WHERE source_ref=?1 AND configuration_revision IS NULL",params![source_ref,revision])?;Ok(())}).await
    }
    pub async fn events(&self, query: EventQuery) -> Result<EventsOverview, StorageError> {
        if query.limit.is_some_and(|n| !(1..=200).contains(&n))
            || query
                .level
                .as_ref()
                .is_some_and(|s| !["info", "warn", "error"].contains(&s.as_str()))
            || query
                .module
                .as_ref()
                .is_some_and(|s| !EVENT_MODULES.contains(&s.as_str()))
            || query.task_id.is_some_and(|n| n <= 0)
            || !text(&query.request_id, 64)
            || !text(&query.node_id, 128)
            || !text(&query.provider_id, 64)
            || query.after_unix_ms.is_some_and(|n| n > i64::MAX as u64)
        {
            return Err(StorageError::Invalid);
        }
        let supplied = query
            .cursor
            .as_ref()
            .map(|s| {
                parse_event_cursor(s)
                    .map(|(e, n)| (e.to_owned(), n))
                    .ok_or(StorageError::Invalid)
            })
            .transpose()?;
        let epoch = self.epoch.clone();
        self.call(move|c|{
            let head:i64=c.query_row("SELECT coalesce((SELECT seq FROM sqlite_sequence WHERE name='service_events'),0)",[],|r|r.get(0))?;
            let oldest:i64=c.query_row("SELECT coalesce(min(sequence),?1+1) FROM service_events",[head],|r|r.get(0))?;
            let reset=supplied.as_ref().and_then(|(e,n)|if e!=&epoch {Some("instance_changed")} else if *n<oldest-1 {Some("cursor_expired")} else if *n>head {Some("cursor_ahead")} else {None});
            let after=if reset.is_none() {supplied.as_ref().map(|(_,n)|*n)} else {None};
            let order=if after.is_some(){"ASC"}else{"DESC"};
            let limit=query.limit.unwrap_or(100);
            let mut records=c.prepare(&format!("SELECT {EVENT_COLUMNS} FROM service_events WHERE sequence<=?1 AND (?2 IS NULL OR sequence>?2) AND (?3 IS NULL OR level=?3) AND (?4 IS NULL OR module=?4) AND (?5 IS NULL OR task_id=?5) AND (?6 IS NULL OR request_id=?6) AND (?7 IS NULL OR node_id=?7) AND (?8 IS NULL OR provider_id=?8) AND (?9 IS NULL OR occurred_at>=?9) ORDER BY sequence {order} LIMIT ?10"))?
                .query_map(params![head,after,query.level,query.module,query.task_id,query.request_id,query.node_id,query.provider_id,query.after_unix_ms.map(|n|n as i64),limit+1],event_row)?.collect::<Result<Vec<_>,_>>()?;
            let more=after.is_some() && records.len()>limit as usize;
            records.truncate(limit as usize);
            let scanned=if more {records.last().map_or(head,|e|e.sequence)}else{head};
            if after.is_none(){records.reverse();}
            Ok(EventsOverview {cursor:event_cursor(&epoch,scanned),epoch,observed_at_unix_ms:unix_ms(),records,oldest_available_sequence:oldest,head_sequence:head,has_more:more,reset_reason:reset.map(str::to_owned),retention_limit:EVENT_RETENTION})
        }).await
    }
}

// Filled by the same projection used in the migration after all accepted run joins exist.
pub(super) fn project_health(c: &Connection, id: i64) -> Result<(), StorageError> {
    c.execute(include_str!("../health-task-projection.sql"), [id])?;
    Ok(())
}
