//! Bounded asynchronous facade over one SQLite owner thread.
pub mod controller;
pub mod health;
pub mod provider;
use fs2::FileExt;
use hps_config::DaemonConfig;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("configuration changed; reload before retrying")]
    Conflict,
    #[error("configuration revision not found")]
    NotFound,
    #[error("invalid or oversized configuration")]
    Invalid,
    #[error("data directory is already owned by another daemon")]
    AlreadyRunning,
    #[error("database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("cannot open data directory or instance lock")]
    Io(#[from] std::io::Error),
    #[error("database schema is newer than this daemon")]
    SchemaTooNew,
    #[error("provider request interval has not elapsed")]
    ProviderRateLimited,
    #[error("provider is paused; inspect controller state before explicitly resuming")]
    ProviderPaused,
    #[error("provider daily extraction budget exhausted")]
    ProviderBudgetExceeded,
    #[error("provider pool storage budget exceeded")]
    PoolFull,
    #[error("health probe queue or daily budget exhausted")]
    HealthCapacity,
    #[error("storage worker is unavailable")]
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub checksum: String,
    pub created_at: i64,
    pub note: String,
    pub applied_at: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct Document {
    pub revision: Revision,
    pub config: DaemonConfig,
}
#[derive(Debug, Clone)]
pub struct Draft {
    pub version: i64,
    pub base_revision: i64,
    pub config: DaemonConfig,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activation {
    pub id: i64,
    pub revision: i64,
    pub previous_revision: i64,
    pub state: String,
    pub created_at: i64,
    pub error_code: Option<String>,
}
#[derive(Debug, Clone)]
pub struct Workspace {
    pub desired: Document,
    pub last_applied: Option<i64>,
    pub draft: Draft,
    pub revisions: Vec<Revision>,
    pub activations: Vec<Activation>,
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
enum Message {
    Job(Job),
    Stop(oneshot::Sender<Result<(), StorageError>>),
}
#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Message>,
    pub database_path: Option<PathBuf>,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn document(config: &DaemonConfig) -> Result<String, StorageError> {
    config.validate().map_err(|_| StorageError::Invalid)?;
    let document = serde_json::to_string(config).map_err(|_| StorageError::Invalid)?;
    if document.len() > 2 * 1024 * 1024 {
        return Err(StorageError::Invalid);
    }
    Ok(document)
}
fn open_directory(path: &Path) -> Result<(Connection, File, PathBuf), StorageError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    let path = path.canonicalize()?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(path.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| StorageError::AlreadyRunning)?;
    let database = path.join("hps.sqlite3");
    drop(options.open(&database)?);
    let connection = Connection::open(&database)?;
    Ok((connection, lock, database))
}

impl Store {
    /// Blocking startup; async callers should use spawn_blocking.
    pub fn open(directory: Option<&Path>, bootstrap: DaemonConfig) -> Result<Self, StorageError> {
        let bootstrap = document(&bootstrap)?;
        let directory = directory.map(Path::to_owned);
        let (tx, mut rx) = mpsc::channel::<Message>(128);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("hps-sqlite-owner".into())
            .spawn(move || {
                let setup = (|| {
                    let (mut connection, lock, path) = if let Some(directory) = directory {
                        let (connection, lock, path) = open_directory(&directory)?;
                        (connection, Some(lock), Some(path))
                    } else {
                        (Connection::open_in_memory()?, None, None)
                    };
                    connection.busy_timeout(Duration::from_secs(3))?;
                    connection.pragma_update(None, "foreign_keys", true)?;
                    if path.is_some() {
                        connection.pragma_update(None, "journal_mode", "WAL")?;
                        connection.pragma_update(None, "synchronous", "FULL")?;
                    }
                    migrate(&mut connection, &bootstrap)?;
                    Ok::<_, StorageError>((connection, lock, path))
                })();
                let (mut connection, lock, path) = match setup {
                    Ok(setup) => setup,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(path)).is_err() {
                    return;
                }
                while let Some(message) = rx.blocking_recv() {
                    match message {
                        Message::Job(job) => job(&mut connection),
                        Message::Stop(done) => {
                            let result=connection.execute("UPDATE provider_clock SET clean_shutdown=1,observed_at=max(observed_at,?1) WHERE singleton=1",[unix_now()]).map(|_|()).map_err(StorageError::from);
                            drop(connection);
                            drop(lock);
                            let _ = done.send(result);
                            return;
                        }
                    }
                }
            })?;
        let database_path = ready_rx.recv().map_err(|_| StorageError::Closed)??;
        Ok(Self { tx, database_path })
    }
    async fn call<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Message::Job(Box::new(move |connection| {
                let _ = tx.send(job(connection));
            })))
            .await
            .map_err(|_| StorageError::Closed)?;
        rx.await.map_err(|_| StorageError::Closed)?
    }
    pub async fn close(&self) -> Result<(), StorageError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Message::Stop(tx))
            .await
            .map_err(|_| StorageError::Closed)?;
        rx.await.map_err(|_| StorageError::Closed)?
    }
    pub async fn workspace(&self) -> Result<Workspace, StorageError> {
        self.call(|connection|{
            let (desired,last_applied):(i64,Option<i64>)=connection.query_row("SELECT desired_revision,last_applied_revision FROM config_head WHERE singleton=1",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
            let draft=connection.query_row("SELECT version,base_revision,document,updated_at FROM config_draft WHERE singleton=1",[],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?)))?;
            let mut revisions=connection.prepare("SELECT id,parent_id,checksum,created_at,note,applied_at FROM config_revisions ORDER BY id DESC LIMIT 50")?;
            let revisions=revisions.query_map([],revision_row)?.collect::<Result<_,_>>()?;
            let mut activations=connection.prepare("SELECT id,revision,previous_revision,state,created_at,error_code FROM config_activations ORDER BY id DESC LIMIT 50")?;
            let activations=activations.query_map([],|row|Ok(Activation{id:row.get(0)?,revision:row.get(1)?,previous_revision:row.get(2)?,state:row.get(3)?,created_at:row.get(4)?,error_code:row.get(5)?}))?.collect::<Result<_,_>>()?;
            Ok(Workspace{desired:read_document(connection,desired)?,last_applied,draft:Draft{version:draft.0,base_revision:draft.1,config:serde_json::from_str(&draft.2).map_err(|_|StorageError::Invalid)?,updated_at:draft.3},revisions,activations})
        }).await
    }
    pub async fn revision(&self, id: i64) -> Result<Document, StorageError> {
        self.call(move |connection| read_document(connection, id))
            .await
    }
    pub async fn prune_versions(&self, maximum: u32, pinned: Vec<i64>) -> Result<(), StorageError> {
        self.call(move|connection|{
            let transaction=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            transaction.execute_batch("CREATE TEMP TABLE IF NOT EXISTS hps_retained_versions(id INTEGER PRIMARY KEY);DELETE FROM hps_retained_versions")?;
            transaction.execute("INSERT INTO hps_retained_versions SELECT id FROM config_revisions ORDER BY id DESC LIMIT ?1",[maximum])?;
            transaction.execute_batch("INSERT OR IGNORE INTO hps_retained_versions SELECT desired_revision FROM config_head;INSERT OR IGNORE INTO hps_retained_versions SELECT last_applied_revision FROM config_head WHERE last_applied_revision IS NOT NULL;INSERT OR IGNORE INTO hps_retained_versions SELECT base_revision FROM config_draft")?;
            for id in pinned {transaction.execute("INSERT OR IGNORE INTO hps_retained_versions VALUES(?1)",[id])?;}
            transaction.execute_batch("DELETE FROM config_activations WHERE revision NOT IN (SELECT id FROM hps_retained_versions) OR previous_revision NOT IN (SELECT id FROM hps_retained_versions);UPDATE config_revisions SET parent_id=NULL WHERE parent_id NOT IN (SELECT id FROM hps_retained_versions);DELETE FROM config_revisions WHERE id NOT IN (SELECT id FROM hps_retained_versions)")?;
            transaction.execute("DELETE FROM config_activations WHERE id NOT IN (SELECT id FROM config_activations ORDER BY id DESC LIMIT ?1)",[maximum.saturating_mul(2)])?;
            transaction.commit()?;Ok(())
        }).await
    }
    pub async fn save_draft(
        &self,
        base: i64,
        expected_version: i64,
        config: DaemonConfig,
    ) -> Result<Draft, StorageError> {
        let body = document(&config)?;
        self.call(move|connection|{
            let transaction=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check_head(&transaction,base)?;
            let changed=transaction.execute("UPDATE config_draft SET version=version+1,base_revision=?1,document=?2,updated_at=?3 WHERE singleton=1 AND version=?4",params![base,body,unix_now(),expected_version])?;
            if changed!=1{return Err(StorageError::Conflict);}
            transaction.commit()?;
            Ok(Draft{version:expected_version+1,base_revision:base,config,updated_at:unix_now()})
        }).await
    }
    pub async fn create_revision(
        &self,
        expected_head: i64,
        config: DaemonConfig,
        note: String,
    ) -> Result<Revision, StorageError> {
        if note.len() > 256 {
            return Err(StorageError::Invalid);
        }
        let body = document(&config)?;
        self.call(move |connection| {
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check_head(&transaction, expected_head)?;
            let id = insert_revision(&transaction, Some(expected_head), &body, &note)?;
            let revision = read_document(&transaction, id)?.revision;
            transaction.commit()?;
            Ok(revision)
        })
        .await
    }
    pub async fn activate(
        &self,
        revision: i64,
        expected_head: i64,
        pending_restart: bool,
    ) -> Result<i64, StorageError> {
        self.call(move|connection|{
            let transaction=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;check_head(&transaction,expected_head)?;read_document(&transaction,revision)?;
            transaction.execute("UPDATE config_head SET desired_revision=?1 WHERE singleton=1",[revision])?;
            transaction.execute("INSERT INTO config_activations(revision,previous_revision,state,created_at) VALUES(?1,?2,?3,?4)",params![revision,expected_head,if pending_restart {"pending_restart"} else {"prepared"},unix_now()])?;
            let activation=transaction.last_insert_rowid();transaction.commit()?;Ok(activation)
        }).await
    }
    pub async fn mark_applied(
        &self,
        revision: i64,
        activation: Option<i64>,
    ) -> Result<(), StorageError> {
        self.call(move|connection|{
            let transaction=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;read_document(&transaction,revision)?;
            transaction.execute("UPDATE config_head SET last_applied_revision=?1 WHERE singleton=1",[revision])?;
            transaction.execute("UPDATE config_revisions SET applied_at=?1 WHERE id=?2",params![unix_now(),revision])?;
            if let Some(id)=activation {transaction.execute("UPDATE config_activations SET state='applied',error_code=NULL WHERE id=?1 AND revision=?2",params![id,revision])?;}
            else {transaction.execute("UPDATE config_activations SET state='applied',error_code=NULL WHERE revision=?1 AND state IN ('pending_restart','prepared','interrupted')",[revision])?;}
            transaction.commit()?;Ok(())
        }).await
    }
    pub async fn record_failure(
        &self,
        revision: i64,
        expected_head: i64,
        code: &'static str,
    ) -> Result<(), StorageError> {
        self.call(move|connection|{
            check_head(connection,expected_head)?;read_document(connection,revision)?;
            connection.execute("INSERT INTO config_activations(revision,previous_revision,state,created_at,error_code) VALUES(?1,?2,'failed',?3,?4)",params![revision,expected_head,unix_now(),code])?;Ok(())
        }).await
    }
}

fn revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Revision> {
    Ok(Revision {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        checksum: row.get(2)?,
        created_at: row.get(3)?,
        note: row.get(4)?,
        applied_at: row.get(5)?,
    })
}
fn read_document(connection: &Connection, id: i64) -> Result<Document, StorageError> {
    let row=connection.query_row("SELECT id,parent_id,checksum,created_at,note,applied_at,document FROM config_revisions WHERE id=?1",[id],|row|Ok((revision_row(row)?,row.get::<_,String>(6)?))).optional()?.ok_or(StorageError::NotFound)?;
    if format!("{:x}", Sha256::digest(row.1.as_bytes())) != row.0.checksum {
        return Err(StorageError::Invalid);
    }
    let config: DaemonConfig = serde_json::from_str(&row.1).map_err(|_| StorageError::Invalid)?;
    config.validate().map_err(|_| StorageError::Invalid)?;
    Ok(Document {
        revision: row.0,
        config,
    })
}
fn insert_revision(
    connection: &Connection,
    parent: Option<i64>,
    body: &str,
    note: &str,
) -> Result<i64, StorageError> {
    connection.execute("INSERT INTO config_revisions(parent_id,document,checksum,created_at,note) VALUES(?1,?2,?3,?4,?5)",params![parent,body,format!("{:x}",Sha256::digest(body.as_bytes())),unix_now(),note])?;
    Ok(connection.last_insert_rowid())
}
fn check_head(connection: &Connection, expected: i64) -> Result<(), StorageError> {
    let actual: i64 = connection.query_row(
        "SELECT desired_revision FROM config_head WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if actual == expected {
        Ok(())
    } else {
        Err(StorageError::Conflict)
    }
}
fn migrate(connection: &mut Connection, bootstrap: &str) -> Result<(), StorageError> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL)")?;
    let version: i64 = connection.query_row(
        "SELECT coalesce(max(version),0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if version > 6 {
        return Err(StorageError::SchemaTooNew);
    }
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if version == 0 {
        transaction.execute_batch(include_str!("../migrations/0001_configuration.up.sql"))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(1,?1)",
            [unix_now()],
        )?;
    }
    if version < 2 {
        transaction.execute_batch(include_str!("../migrations/0002_known_good.up.sql"))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(2,?1)",
            [unix_now()],
        )?;
    }
    if version < 3 {
        transaction.execute_batch(include_str!("../migrations/0003_provider_leases.up.sql"))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(3,?1)",
            [unix_now()],
        )?;
    }
    transaction.execute("UPDATE provider_tasks SET state=CASE WHEN state='accepted' THEN 'failed' ELSE 'outcome_unknown' END,finished_at=?1,error_code='process_interrupted' WHERE state IN ('accepted','requesting')",[unix_now()])?;
    if version < 4 {
        transaction.execute_batch(include_str!("../migrations/0004_lease_clock.up.sql"))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(4,?1)",
            [unix_now()],
        )?;
    }
    if version < 5 {
        transaction.execute_batch(include_str!(
            "../migrations/0005_provider_controller.up.sql"
        ))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(5,?1)",
            [unix_now()],
        )?;
    }
    if version < 6 {
        transaction.execute_batch(include_str!("../migrations/0006_health.up.sql"))?;
        transaction.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(6,?1)",
            [unix_now()],
        )?;
    }
    transaction.execute("UPDATE health_runs SET state='interrupted',finished_at=?1 WHERE state IN ('queued','probing')",[unix_now()])?;
    transaction
        .execute_batch("CREATE INDEX IF NOT EXISTS health_state_age ON health_state(checked_at)")?;
    // Fresh proof is reacquired after every process start; persisted results remain diagnostic history.
    transaction.execute("UPDATE health_state SET result=json_set(result,'$.verdict','unknown','$.error_code','process_restarted'),next_due_at=?1 WHERE json_extract(result,'$.verdict')='healthy'",[unix_now()])?;
    transaction.execute("UPDATE provider_control SET paused_reason='process_interrupted' WHERE last_task_id IN (SELECT id FROM provider_tasks WHERE state='outcome_unknown' AND error_code='process_interrupted' AND (provider_control.acknowledged_at IS NULL OR provider_control.acknowledged_at<provider_tasks.finished_at))",[])?;
    let observed: Option<(i64, bool)> = transaction
        .query_row(
            "SELECT observed_at,clean_shutdown FROM provider_clock WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if observed.is_some_and(|(time, _)| unix_now() < time) {
        transaction.execute("DELETE FROM health_state", [])?;
        transaction.execute(
            "UPDATE provider_leases SET state='clock_rollback' WHERE state='active'",
            [],
        )?;
    } else if observed.is_some_and(|(_, clean)| !clean) {
        // A crashed process cannot prove elapsed monotonic time. Retire instead of extending paid leases.
        transaction.execute(
            "UPDATE provider_leases SET state='revoked' WHERE state='active'",
            [],
        )?;
    }
    transaction.execute("INSERT INTO provider_clock(singleton,observed_at,clean_shutdown) VALUES(1,?1,0) ON CONFLICT(singleton) DO UPDATE SET observed_at=max(observed_at,excluded.observed_at),clean_shutdown=0",[unix_now()])?;
    let count: i64 =
        transaction.query_row("SELECT count(*) FROM config_head", [], |row| row.get(0))?;
    if count == 0 {
        let id = insert_revision(&transaction, None, bootstrap, "bootstrap")?;
        transaction.execute(
            "INSERT INTO config_head(singleton,desired_revision) VALUES(1,?1)",
            [id],
        )?;
        transaction.execute("INSERT INTO config_draft(singleton,version,base_revision,document,updated_at) VALUES(1,1,?1,?2,?3)",params![id,bootstrap,unix_now()])?;
    }
    transaction.execute("UPDATE config_activations SET state='interrupted',error_code='process_exited_before_confirmation' WHERE state='prepared'",[])?;
    transaction.commit()?;
    Ok(())
}
