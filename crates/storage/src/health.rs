//! Durable, bounded probe reservations. Supplier accounting is independent.
use super::*;
use hps_domain::{HealthRun, HealthTask, HealthVerdict, ProbeResult};

#[derive(Clone, Debug)]
pub struct ProbeReservation {
    pub node_id: String,
    pub identity: String,
    pub profile_fingerprint: String,
}
#[derive(Clone, Debug)]
pub struct HealthState {
    pub result: ProbeResult,
    pub failures: u32,
    pub next_due_at: i64,
}
#[derive(Clone, Debug)]
pub struct HealthUsage {
    pub queued: u32,
    pub probing: u32,
    pub day: i64,
    pub used: u32,
}
const RUN_COLUMNS: &str =
    "id,node_id,identity,profile_fingerprint,state,created_at,started_at,finished_at,result";
fn run(row: &rusqlite::Row<'_>) -> rusqlite::Result<HealthRun> {
    let document: Option<String> = row.get(8)?;
    Ok(HealthRun {
        id: row.get(0)?,
        node_id: row.get(1)?,
        identity: row.get(2)?,
        profile_fingerprint: row.get(3)?,
        state: row.get(4)?,
        created_at: row.get(5)?,
        started_at: row.get(6)?,
        finished_at: row.get(7)?,
        result: document
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
    })
}
fn task(c: &Connection, id: i64) -> Result<HealthTask, StorageError> {
    let (request_id, trigger, created_at, merged) = c
        .query_row(
            "SELECT request_id,trigger,created_at,merged FROM health_tasks WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(StorageError::NotFound)?;
    let runs=c.prepare(&format!("SELECT {RUN_COLUMNS} FROM health_runs WHERE id IN (SELECT run_id FROM health_task_runs WHERE task_id=?1) ORDER BY id"))?.query_map([id],run)?.collect::<Result<Vec<_>,_>>()?;
    let completed = runs
        .iter()
        .filter(|r| !matches!(r.state.as_str(), "queued" | "probing"))
        .count() as u32;
    let healthy = runs
        .iter()
        .filter(|r| {
            r.result
                .as_ref()
                .is_some_and(|r| r.verdict == HealthVerdict::Healthy)
        })
        .count() as u32;
    let skipped = runs
        .iter()
        .filter(|r| {
            r.result.as_ref().is_some_and(|r| {
                matches!(
                    r.verdict,
                    HealthVerdict::Skipped | HealthVerdict::Expired | HealthVerdict::Unsupported
                )
            }) || r.state == "discarded"
        })
        .count() as u32;
    let state = if completed < runs.len() as u32 {
        "running"
    } else if runs.iter().any(|r| r.state == "interrupted") {
        "interrupted"
    } else {
        "completed"
    };
    Ok(HealthTask {
        id,
        request_id,
        trigger,
        state: state.into(),
        created_at,
        merged,
        total: runs.len() as u32,
        completed,
        healthy,
        failed: completed.saturating_sub(healthy + skipped),
        skipped,
        runs,
    })
}
impl Store {
    pub async fn begin_health_task(
        &self,
        request_id: String,
        trigger: String,
        probes: Vec<ProbeReservation>,
        queue_capacity: usize,
        daily_budget: u32,
    ) -> Result<HealthTask, StorageError> {
        if request_id.is_empty()
            || request_id.len() > 64
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !["manual", "automatic"].contains(&trigger.as_str())
            || probes.len() > 4000
        {
            return Err(StorageError::Invalid);
        }
        self.call(move|c|{
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if let Some(id)=tx.query_row("SELECT id FROM health_tasks WHERE request_id=?1",[&request_id],|r|r.get(0)).optional()? {return task(&tx,id);}
            let tasks:u32=tx.query_row("SELECT count(*) FROM health_tasks",[],|r|r.get(0))?;
            if tasks>=5000 {
                tx.execute("DELETE FROM health_tasks WHERE id IN (SELECT id FROM health_tasks WHERE id NOT IN (SELECT task_id FROM health_task_runs JOIN health_runs ON run_id=health_runs.id WHERE state IN ('queued','probing')) ORDER BY id LIMIT ?1)",[tasks-4999])?;
                let remaining:u32=tx.query_row("SELECT count(*) FROM health_tasks",[],|r|r.get(0))?;
                if remaining>=5000{return Err(StorageError::HealthCapacity);}
            }
            let now=unix_now();let day=super::controller::budget_day(&tx,now)?;
            let used:u32=tx.query_row("SELECT probes FROM health_daily_usage WHERE day=?1",[day],|r|r.get(0)).optional()?.unwrap_or(0);
            let inflight:u32=tx.query_row("SELECT count(*) FROM health_runs WHERE state IN ('queued','probing')",[],|r|r.get(0))?;
            let mut items=Vec::new();let mut new=0u32;
            let mut seen=std::collections::HashSet::new();
            for probe in probes {
                if !seen.insert((probe.identity.clone(),probe.profile_fingerprint.clone())) {continue;}
                let existing:Option<i64>=tx.query_row("SELECT id FROM health_runs WHERE identity=?1 AND profile_fingerprint=?2 AND state IN ('queued','probing')",params![probe.identity,probe.profile_fingerprint],|r|r.get(0)).optional()?;
                if existing.is_none(){new+=1;} items.push((probe,existing));
            }
            if inflight as usize+new as usize>queue_capacity || used.saturating_add(new)>daily_budget {return Err(StorageError::HealthCapacity);}
            tx.execute("INSERT INTO health_tasks(request_id,trigger,created_at,merged) VALUES(?1,?2,?3,?4)",params![request_id,trigger,now,items.len() as u32-new])?;
            let task_id=tx.last_insert_rowid();
            for(probe,existing)in items {
                let id=if let Some(id)=existing{id}else{
                    tx.execute("INSERT INTO health_runs(node_id,identity,profile_fingerprint,state,created_at) VALUES(?1,?2,?3,'queued',?4)",params![probe.node_id,probe.identity,probe.profile_fingerprint,now])?;tx.last_insert_rowid()
                };
                tx.execute("INSERT INTO health_task_runs(task_id,run_id) VALUES(?1,?2)",params![task_id,id])?;
            }
            tx.execute("INSERT INTO health_daily_usage(day,probes) VALUES(?1,?2) ON CONFLICT(day) DO UPDATE SET probes=probes+excluded.probes",params![day,new])?;
            tx.execute("UPDATE provider_clock SET observed_at=max(observed_at,?1) WHERE singleton=1",[now])?;
            tx.execute("DELETE FROM health_daily_usage WHERE day<?1",[day-400])?;
            tx.execute("DELETE FROM health_tasks WHERE id NOT IN (SELECT task_id FROM health_task_runs JOIN health_runs ON run_id=health_runs.id WHERE state IN ('queued','probing')) AND (id NOT IN (SELECT id FROM health_tasks ORDER BY id DESC LIMIT 5000) OR id IN (SELECT task_id FROM health_task_runs WHERE run_id NOT IN (SELECT id FROM health_runs ORDER BY id DESC LIMIT 100000)))",[])?;
            tx.execute("DELETE FROM health_runs WHERE state NOT IN ('queued','probing') AND id NOT IN (SELECT run_id FROM health_task_runs)",[])?;
            tx.execute("DELETE FROM health_state WHERE checked_at<?1 OR rowid IN (SELECT rowid FROM health_state ORDER BY checked_at DESC LIMIT -1 OFFSET 400000)",[now-30*86400])?;
            super::tasks::project_health(&tx,task_id)?;
            let result=task(&tx,task_id)?;tx.commit()?;Ok(result)
        }).await
    }
    pub async fn claim_health_run(&self, id: i64) -> Result<bool, StorageError> {
        self.call(move|c|Ok(c.execute("UPDATE health_runs SET state='probing',started_at=?2 WHERE id=?1 AND state='queued'",params![id,unix_now()])?==1)).await
    }
    pub async fn queued_health_runs(&self, limit: u32) -> Result<Vec<HealthRun>, StorageError> {
        self.call(move |c| {
            Ok(c.prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM health_runs WHERE state='queued' ORDER BY id LIMIT ?1"
            ))?
            .query_map([limit.min(64)], run)?
            .collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }
    pub async fn discard_health_run(&self, id: i64) -> Result<(), StorageError> {
        self.call(move|c|{c.execute("UPDATE health_runs SET state='discarded',finished_at=?2 WHERE id=?1 AND state IN ('queued','probing')",params![id,unix_now()])?;Ok(())}).await
    }
    pub async fn finish_health_run(
        &self,
        id: i64,
        result: ProbeResult,
        next_due_at: i64,
        failures: u32,
    ) -> Result<(), StorageError> {
        self.call(move|c|{let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let document=serde_json::to_string(&result).map_err(|_|StorageError::Invalid)?;
            if tx.execute("UPDATE health_runs SET state='completed',finished_at=?2,result=?3 WHERE id=?1 AND state='probing' AND identity=?4 AND profile_fingerprint=?5",params![id,unix_now(),document,result.identity,result.profile_fingerprint])?!=1 {return Err(StorageError::Conflict);}
            tx.execute("INSERT INTO health_state(identity,profile_fingerprint,result,checked_at,failures,next_due_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(identity,profile_fingerprint) DO UPDATE SET result=excluded.result,checked_at=excluded.checked_at,failures=excluded.failures,next_due_at=excluded.next_due_at WHERE excluded.checked_at>=health_state.checked_at",params![result.identity,result.profile_fingerprint,document,result.checked_at,failures,next_due_at])?;
            let states:u32=tx.query_row("SELECT count(*) FROM health_state",[],|r|r.get(0))?;
            if states>400000 {tx.execute("DELETE FROM health_state WHERE rowid IN (SELECT rowid FROM health_state ORDER BY checked_at ASC LIMIT ?1)",[states-400000])?;}
            tx.commit()?;Ok(())
        }).await
    }
    pub async fn health_states(&self) -> Result<Vec<HealthState>, StorageError> {
        self.call(|c|{
        let rows=c.prepare("SELECT result,failures,next_due_at FROM health_state ORDER BY checked_at DESC LIMIT 400000")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,i64>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        rows.into_iter().map(|(doc,failures,next_due_at)|Ok(HealthState{result:serde_json::from_str(&doc).map_err(|_|StorageError::Invalid)?,failures,next_due_at})).collect()
    }).await
    }
    pub async fn health_task(&self, id: i64) -> Result<HealthTask, StorageError> {
        self.call(move |c| task(c, id)).await
    }
    pub async fn health_tasks(&self) -> Result<Vec<HealthTask>, StorageError> {
        self.call(|c| {
            let ids = c
                .prepare("SELECT id FROM health_tasks ORDER BY id DESC LIMIT 30")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.into_iter().map(|id| task_summary(c, id)).collect()
        })
        .await
    }
    pub async fn health_usage(&self) -> Result<HealthUsage, StorageError> {
        self.call(|c|{let day=super::controller::budget_day(c,unix_now())?;let (queued,probing)=c.query_row("SELECT coalesce(sum(state='queued'),0),coalesce(sum(state='probing'),0) FROM health_runs WHERE state IN ('queued','probing')",[],|r|Ok((r.get(0)?,r.get(1)?)))?;let used=c.query_row("SELECT probes FROM health_daily_usage WHERE day=?1",[day],|r|r.get(0)).optional()?.unwrap_or(0);Ok(HealthUsage{queued,probing,day,used})}).await
    }
}
impl Store {
    pub async fn invalidate_health(&self, state: HealthState) -> Result<(), StorageError> {
        self.call(move|c|{let result=state.result;let doc=serde_json::to_string(&result).map_err(|_|StorageError::Invalid)?;c.execute("UPDATE health_state SET result=?3,checked_at=?4,failures=max(failures,?5),next_due_at=?6 WHERE identity=?1 AND profile_fingerprint=?2 AND checked_at<=?4",params![result.identity,result.profile_fingerprint,doc,result.checked_at,state.failures,state.next_due_at])?;Ok(())}).await
    }
}
impl Store {
    pub async fn pending_health_runs(&self) -> Result<Vec<HealthRun>, StorageError> {
        self.call(|c|Ok(c.prepare(&format!("SELECT {RUN_COLUMNS} FROM health_runs WHERE state IN ('queued','probing') ORDER BY id LIMIT 10000"))?.query_map([],run)?.collect::<Result<Vec<_>,_>>()?)).await
    }
    pub async fn health_task_by_request(
        &self,
        request: String,
    ) -> Result<Option<HealthTask>, StorageError> {
        self.call(move |c| {
            c.query_row(
                "SELECT id FROM health_tasks WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )
            .optional()?
            .map(|id| task(c, id))
            .transpose()
        })
        .await
    }
}
fn task_summary(c: &Connection, id: i64) -> Result<HealthTask, StorageError> {
    let (request_id, trigger, created_at, merged) = c.query_row(
        "SELECT request_id,trigger,created_at,merged FROM health_tasks WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let(total,completed,healthy,skipped,inflight,interrupted):(u32,u32,u32,u32,u32,u32)=c.query_row("SELECT count(*),coalesce(sum(state NOT IN ('queued','probing')),0),coalesce(sum(json_extract(result,'$.verdict')='healthy'),0),coalesce(sum(state='discarded' OR json_extract(result,'$.verdict') IN ('skipped','expired','unsupported')),0),coalesce(sum(state IN ('queued','probing')),0),coalesce(sum(state='interrupted'),0) FROM health_runs WHERE id IN (SELECT run_id FROM health_task_runs WHERE task_id=?1)",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    Ok(HealthTask {
        id,
        request_id,
        trigger,
        created_at,
        merged,
        total,
        completed,
        healthy,
        failed: completed.saturating_sub(healthy + skipped),
        skipped,
        state: if inflight > 0 {
            "running"
        } else if interrupted > 0 {
            "interrupted"
        } else {
            "completed"
        }
        .into(),
        runs: vec![],
    })
}
