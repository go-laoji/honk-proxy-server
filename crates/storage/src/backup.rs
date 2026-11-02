//! Consistent SQLite snapshots and explicitly offline, locked restore.
use super::*;
use rusqlite::{
    OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    io::{Read, Write},
    time::Instant,
};

pub const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
pub const SCHEMA_VERSION: i64 = 8;
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup requires a persistent data directory")]
    Ephemeral,
    #[error("backup capacity reached; retry after the current snapshot finishes")]
    Busy,
    #[error("backup is corrupt, unsupported, oversized, or has an unexpected digest")]
    Invalid,
    #[error("backup page copy exceeded the 30 second deadline")]
    Timeout,
    #[error("stop the daemon before restoring; its data directory is locked")]
    Running,
    #[error("SQLite backup operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("backup file operation failed")]
    Io(#[from] std::io::Error),
    #[error(
        "restore was published but directory durability could not be confirmed; keep the daemon stopped and inspect hps.pre-restore files before retrying"
    )]
    PublishedDurability,
    #[error("backup worker unavailable")]
    Worker,
}
pub struct Snapshot {
    pub metadata: hps_domain::BackupMetadata,
    pub bytes: Vec<u8>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub restored: hps_domain::BackupMetadata,
    pub preserved_database: Option<PathBuf>,
    pub restart_required: bool,
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn private_file(path: &Path) -> Result<File, BackupError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn readonly(path: &Path) -> Result<Connection, BackupError> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}
fn validate(connection: &Connection) -> Result<i64, BackupError> {
    connection.pragma_update(None, "trusted_schema", false)?;
    // Authenticate the version table's own shape before executing any SQL from it.
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(SCHEMA_MIGRATIONS_SQL)?;
    let metadata_object = |c: &Connection| -> Result<SchemaObject, rusqlite::Error> {
        c.query_row(
            "SELECT name,type,tbl_name,sql FROM sqlite_schema WHERE name='schema_migrations'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
    };
    if metadata_object(connection)? != metadata_object(&reference)? {
        return Err(BackupError::Invalid);
    }
    let version: i64 =
        connection.query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(BackupError::Invalid);
    }
    let scripts = [
        include_str!("../migrations/0001_configuration.up.sql"),
        include_str!("../migrations/0002_known_good.up.sql"),
        include_str!("../migrations/0003_provider_leases.up.sql"),
        include_str!("../migrations/0004_lease_clock.up.sql"),
        include_str!("../migrations/0005_provider_controller.up.sql"),
        include_str!("../migrations/0006_health.up.sql"),
        include_str!("../migrations/0007_node_controls.up.sql"),
        include_str!("../migrations/0008_tasks_events.up.sql"),
    ];
    for script in scripts.iter().take(version as usize) {
        reference.execute_batch(script)?;
    }
    let expected = schema_objects(&reference)?;
    let actual_count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
        [],
        |r| r.get(0),
    )?;
    if actual_count != expected.len() as i64 || schema_objects(connection)? != expected {
        return Err(BackupError::Invalid);
    }
    let check: String = connection.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok"
        || connection
            .prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_some()
    {
        return Err(BackupError::Invalid);
    }
    // Match Store startup's semantic integrity checks, including historical/known-good revisions.
    let mut revisions = connection.prepare("SELECT document,checksum FROM config_revisions")?;
    for row in revisions.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (document, checksum) = row?;
        if format!("{:x}", Sha256::digest(document.as_bytes())) != checksum {
            return Err(BackupError::Invalid);
        }
        let config: DaemonConfig =
            serde_json::from_str(&document).map_err(|_| BackupError::Invalid)?;
        config.validate().map_err(|_| BackupError::Invalid)?;
    }
    let _:i64=connection.query_row("SELECT r.id FROM config_revisions r JOIN config_head h ON r.id=h.desired_revision WHERE h.singleton=1",[],|r|r.get(0))?;
    let draft: String = connection.query_row(
        "SELECT document FROM config_draft WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let config: DaemonConfig = serde_json::from_str(&draft).map_err(|_| BackupError::Invalid)?;
    config.validate().map_err(|_| BackupError::Invalid)?;
    if version >= 3 {
        let observed: i64 = connection.query_row(
            "SELECT observed_at FROM provider_clock WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if observed < 0 {
            return Err(BackupError::Invalid);
        }
        validate_documents::<hps_provider::LeaseCandidate>(
            connection,
            "SELECT document FROM provider_leases",
        )?;
    }
    if version >= 6 {
        validate_documents::<hps_domain::ProbeResult>(
            connection,
            "SELECT result FROM health_state",
        )?;
        validate_documents::<hps_domain::ProbeResult>(
            connection,
            "SELECT result FROM health_runs WHERE result IS NOT NULL",
        )?;
    }
    if version >= 8 {
        let epoch: String = connection.query_row(
            "SELECT epoch FROM event_context WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if !hps_domain::valid_connection_epoch(&epoch) {
            return Err(BackupError::Invalid);
        }
    }
    Ok(version)
}
type SchemaObject = (String, String, String, Option<String>);
fn schema_objects(connection: &Connection) -> Result<Vec<SchemaObject>, BackupError> {
    let mut statement=connection.prepare("SELECT name,type,tbl_name,sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY name,type,tbl_name,sql")?;
    Ok(statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?)
}
fn validate_documents<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    sql: &str,
) -> Result<(), BackupError> {
    let mut statement = connection.prepare(sql)?;
    for row in statement.query_map([], |r| r.get::<_, String>(0))? {
        serde_json::from_str::<T>(&row?).map_err(|_| BackupError::Invalid)?;
    }
    Ok(())
}
fn copy(source: &Connection, destination: &Path) -> Result<(), BackupError> {
    // Pin a WAL read snapshot so a continuously writing source cannot restart every step.
    // This holds no writer mutex; the bounded deadline limits checkpoint retention.
    source.execute_batch("BEGIN DEFERRED")?;
    let _: i64 = source.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
    let pages: i64 = source.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let page_size: i64 = source.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    if pages <= 0 || page_size <= 0 || pages.saturating_mul(page_size) > MAX_BACKUP_BYTES as i64 {
        return Err(BackupError::Invalid);
    }
    let file = private_file(destination)?;
    let mut target = Connection::open(destination)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let backup = Backup::new(source, &mut target)?;
    loop {
        if Instant::now() >= deadline {
            return Err(BackupError::Timeout);
        }
        match backup.step(128)? {
            StepResult::Done => break,
            StepResult::More | StepResult::Busy | StepResult::Locked => {
                std::thread::sleep(Duration::from_millis(5))
            }
            _ => return Err(BackupError::Invalid),
        }
        if file.metadata()?.len() > MAX_BACKUP_BYTES {
            return Err(BackupError::Invalid);
        }
    }
    drop(backup);
    target.pragma_update(None, "journal_mode", "DELETE")?;
    validate(&target)?;
    drop(target);
    file.sync_all()?;
    Ok(())
}
struct CapturedBackup {
    bytes: Vec<u8>,
    sha256: String,
}
struct SealedBackup {
    directory: PathBuf,
    path: PathBuf,
}
impl Drop for SealedBackup {
    fn drop(&mut self) {
        for path in [
            &self.path,
            &self.path.with_extension("sqlite3-wal"),
            &self.path.with_extension("sqlite3-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}
struct VerifiedBackup {
    sealed: SealedBackup,
    metadata: hps_domain::BackupMetadata,
}
/// Read the caller's pathname exactly once. Every later SQL operation uses these captured bytes.
fn capture(path: &Path, expected_sha256: Option<&str>) -> Result<CapturedBackup, BackupError> {
    let mut file = File::open(path)?;
    let info = file.metadata()?;
    let size = info.len();
    if !info.is_file() || size == 0 || size > MAX_BACKUP_BYTES {
        return Err(BackupError::Invalid);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    Read::by_ref(&mut file)
        .take(MAX_BACKUP_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    if bytes.len() as u64 != size || expected_sha256.is_some_and(|expected| expected != sha256) {
        return Err(BackupError::Invalid);
    }
    Ok(CapturedBackup { bytes, sha256 })
}
fn verify_captured(captured: CapturedBackup) -> Result<VerifiedBackup, BackupError> {
    let directory =
        std::env::temp_dir().join(format!("hps-backup-sealed-{}", uuid::Uuid::new_v4()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let sealed = SealedBackup {
        path: directory.join("input.sqlite3"),
        directory,
    };
    let mut file = private_file(&sealed.path)?;
    file.write_all(&captured.bytes)?;
    file.sync_all()?;
    drop(file);
    let connection = readonly(&sealed.path)?;
    let journal: String = connection.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if journal != "delete" {
        return Err(BackupError::Invalid);
    }
    let schema_version = validate(&connection)?;
    let metadata = hps_domain::BackupMetadata {
        format_version: 1,
        schema_version,
        size_bytes: captured.bytes.len() as u64,
        sha256: captured.sha256,
        verified_at_unix: unix_now(),
        contains_credentials: true,
        restore_requires_stopped_daemon: true,
    };
    Ok(VerifiedBackup { sealed, metadata })
}
pub fn inspect(
    path: &Path,
    expected_sha256: Option<&str>,
) -> Result<hps_domain::BackupMetadata, BackupError> {
    Ok(verify_captured(capture(path, expected_sha256)?)?.metadata)
}
fn restore_lock(directory: &Path) -> Result<File, BackupError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(directory.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| BackupError::Running)?;
    Ok(lock)
}
/// Local CLI only. Holds exactly the daemon's lifetime lock until publication completes.
pub fn restore(
    directory: &Path,
    source: &Path,
    expected_sha256: &str,
) -> Result<RestoreResult, BackupError> {
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(BackupError::Invalid);
    }
    let directory = directory.canonicalize()?;
    let lock = restore_lock(&directory)?;
    if source.canonicalize()? == directory.join("hps.sqlite3") {
        return Err(BackupError::Invalid);
    }
    restore_captured(&directory, capture(source, Some(expected_sha256))?, lock)
}
fn restore_captured(
    directory: &Path,
    captured: CapturedBackup,
    lock: File,
) -> Result<RestoreResult, BackupError> {
    let verified = verify_captured(captured)?;
    let original = directory.join("hps.sqlite3");
    let temporary = Temporary(directory.join(format!(".restore-{}.sqlite3", uuid::Uuid::new_v4())));
    copy(&readonly(&verified.sealed.path)?, &temporary.0)?;
    let metadata = inspect(&temporary.0, None)?;
    let preserved = if original.exists() {
        let current = Connection::open(&original)?;
        let (busy, _, _): (i64, i64, i64) =
            current.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if busy != 0 {
            return Err(BackupError::Running);
        }
        drop(current);
        let retained = directory.join(format!("hps.pre-restore-{}.sqlite3", uuid::Uuid::new_v4()));
        std::fs::hard_link(&original, &retained)?;
        Some(retained)
    } else {
        None
    };
    // The original stays present until atomic publication; the hard link preserves recovery data.
    std::fs::rename(&temporary.0, &original)?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|v| v.sync_all())
        .map_err(|_| BackupError::PublishedDurability)?;
    drop(lock);
    Ok(RestoreResult {
        restored: metadata,
        preserved_database: preserved,
        restart_required: true,
    })
}
/// Publish downloaded bytes without overwriting an existing file, with owner-only permissions.
pub fn save_new(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let mut file = private_file(path)?;
    if let Err(error) = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })() {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}
impl Store {
    pub async fn backup(&self) -> Result<Snapshot, BackupError> {
        let permit = self
            .backup_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| BackupError::Busy)?;
        let path = self.database_path.clone().ok_or(BackupError::Ephemeral)?;
        tokio::task::spawn_blocking(move || {
            let temporary =
                Temporary(path.with_file_name(format!(".backup-{}.sqlite3", uuid::Uuid::new_v4())));
            copy(&readonly(&path)?, &temporary.0)?;
            let metadata = inspect(&temporary.0, None)?;
            let bytes = std::fs::read(&temporary.0)?;
            Ok(Snapshot {
                metadata,
                bytes,
                _permit: permit,
            })
        })
        .await
        .map_err(|_| BackupError::Worker)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wal_writers_continue_while_snapshot_is_consistent_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let writer = Connection::open(store.database_path.as_ref().unwrap()).unwrap();
        writer.execute_batch("WITH RECURSIVE fixture(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM fixture WHERE x<10000) INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code) SELECT '00000000-0000-0000-0000-000000000000',0,'info','backup_fixture',printf('%096d',x) FROM fixture").unwrap();
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let signal = running.clone();
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count = progress.clone();
        let worker = std::thread::spawn(move || {
            let mut n = 1;
            while signal.load(std::sync::atomic::Ordering::Relaxed) {
                writer
                    .execute("UPDATE config_draft SET version=?1,updated_at=?1", [n])
                    .unwrap();
                n += 1;
                count.store(n as u64, std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(1));
            }
            n
        });
        while progress.load(std::sync::atomic::Ordering::Relaxed) < 3 {
            std::thread::sleep(Duration::from_millis(1));
        }
        let before = progress.load(std::sync::atomic::Ordering::Relaxed);
        let snapshot = store.backup().await.unwrap();
        assert!(progress.load(std::sync::atomic::Ordering::Relaxed) > before);
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(worker.join().unwrap() > 0);
        let path = dir.path().join("export.sqlite3");
        save_new(&path, &snapshot.bytes).unwrap();
        let actual = inspect(&path, Some(&snapshot.metadata.sha256)).unwrap();
        assert_eq!(actual.schema_version, SCHEMA_VERSION);
        let copy = readonly(&path).unwrap();
        let (a, b): (i64, i64) = copy
            .query_row(
                "SELECT version,updated_at FROM config_draft WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(a, b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(save_new(&path, &snapshot.bytes).is_err());
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".backup-")
        }));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn extra_objects_including_literal_prefix_bypass_cannot_poison_restored_schema() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = store.backup().await.unwrap();
        store.close().await.unwrap();
        let target = dir.path().join("hps.sqlite3");
        let before = std::fs::read(&target).unwrap();
        let source = dir.path().join("untrusted.sqlite3");
        for sql in [
            "CREATE TRIGGER poison_event_context AFTER UPDATE ON event_context BEGIN UPDATE schema_migrations SET version=999; END",
            "CREATE TRIGGER sqliteXpoison AFTER UPDATE ON event_context BEGIN UPDATE schema_migrations SET version=999; END",
            "CREATE TABLE extra_table(payload TEXT)",
            "CREATE VIEW extra_view AS SELECT id FROM config_revisions",
            "CREATE INDEX extra_index ON config_revisions(created_at)",
        ] {
            std::fs::write(&source, &snapshot.bytes).unwrap();
            let db = Connection::open(&source).unwrap();
            db.execute_batch(sql).unwrap();
            drop(db);
            let injected = std::fs::read(&source).unwrap();
            let digest = format!("{:x}", Sha256::digest(&injected));
            assert!(inspect(&source, Some(&digest)).is_err(), "accepted {sql}");
            assert!(
                restore(dir.path(), &source, &digest).is_err(),
                "restored {sql}"
            );
            assert_eq!(before, std::fs::read(&target).unwrap());
            assert_eq!(injected, std::fs::read(&source).unwrap());
            assert_eq!(
                readonly(&target)
                    .unwrap()
                    .query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
        }
    }
    #[tokio::test]
    async fn captured_bytes_bind_digest_schema_and_restore_after_source_path_replacement() {
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let config_a = DaemonConfig {
            instance_name: "captured-A".into(),
            ..Default::default()
        };
        let a = Store::open(Some(a_dir.path()), config_a).unwrap();
        let snapshot_a = a.backup().await.unwrap();
        a.close().await.unwrap();
        let config_b = DaemonConfig {
            instance_name: "replacement-B".into(),
            ..Default::default()
        };
        let b = Store::open(Some(b_dir.path()), config_b).unwrap();
        let snapshot_b = b.backup().await.unwrap();
        b.close().await.unwrap();
        let source = a_dir.path().join("selected.sqlite3");
        save_new(&source, &snapshot_a.bytes).unwrap();
        // Both public inspect and public restore use this same capture -> verify path.
        let for_inspect = capture(&source, Some(&snapshot_a.metadata.sha256)).unwrap();
        let for_restore = capture(&source, Some(&snapshot_a.metadata.sha256)).unwrap();
        std::fs::write(&source, &snapshot_b.bytes).unwrap();
        let verified = verify_captured(for_inspect).unwrap();
        assert_eq!(verified.metadata.sha256, snapshot_a.metadata.sha256);
        assert_eq!(
            std::fs::read(&verified.sealed.path).unwrap(),
            snapshot_a.bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&verified.sealed.directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&verified.sealed.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let sealed_path = verified.sealed.path.clone();
        let sealed_directory = verified.sealed.directory.clone();
        drop(verified);
        assert!(!sealed_path.exists() && !sealed_directory.exists());
        let restored = restore_captured(
            target_dir.path(),
            for_restore,
            restore_lock(target_dir.path()).unwrap(),
        )
        .unwrap();
        assert!(restored.restart_required);
        let target = readonly(&target_dir.path().join("hps.sqlite3")).unwrap();
        let document: String = target
            .query_row(
                "SELECT document FROM config_revisions WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(document.contains("captured-A") && !document.contains("replacement-B"));
        assert_eq!(std::fs::read(&source).unwrap(), snapshot_b.bytes);
        // Replacing the path cannot rescue invalid bytes captured before the replacement either.
        std::fs::write(&source, b"not a SQLite snapshot").unwrap();
        let captured_invalid = capture(&source, None).unwrap();
        std::fs::write(&source, &snapshot_a.bytes).unwrap();
        assert!(verify_captured(captured_invalid).is_err());
    }
    #[tokio::test]
    async fn supported_old_schema_restores_without_forging_version_and_migrates_on_restart() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("legacy.sqlite3");
        let db = Connection::open(&source).unwrap();
        db.execute_batch(include_str!("../migrations/0001_configuration.up.sql"))
            .unwrap();
        db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL);INSERT INTO schema_migrations VALUES(1,0)").unwrap();
        let body = document(&DaemonConfig::default()).unwrap();
        let id = insert_revision(&db, None, &body, "legacy").unwrap();
        db.execute(
            "INSERT INTO config_head(singleton,desired_revision) VALUES(1,?1)",
            [id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO config_draft VALUES(1,1,?1,?2,0)",
            params![id, body],
        )
        .unwrap();
        drop(db);
        let metadata = inspect(&source, None).unwrap();
        assert_eq!(metadata.schema_version, 1);
        let destination = tempfile::tempdir().unwrap();
        let restored = restore(destination.path(), &source, &metadata.sha256).unwrap();
        assert_eq!(restored.restored.schema_version, 1);
        assert!(restored.preserved_database.is_none());
        let store = Store::open(Some(destination.path()), DaemonConfig::default()).unwrap();
        assert_eq!(store.workspace().await.unwrap().desired.revision.id, 1);
        store.close().await.unwrap();
        assert_eq!(
            readonly(&destination.path().join("hps.sqlite3"))
                .unwrap()
                .query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
    }
    #[tokio::test]
    async fn missing_singletons_or_invalid_draft_cannot_replace_a_working_database() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = store.backup().await.unwrap();
        store.close().await.unwrap();
        let target = dir.path().join("hps.sqlite3");
        let before = std::fs::read(&target).unwrap();
        for sql in [
            "DELETE FROM config_draft",
            "UPDATE config_draft SET document='{\"unknown_setting\":true}'",
            "DELETE FROM config_head",
            "DELETE FROM event_context",
            "DELETE FROM provider_clock",
            "DROP TABLE config_activations",
        ] {
            let source = dir.path().join("damaged.sqlite3");
            std::fs::write(&source, &snapshot.bytes).unwrap();
            let db = Connection::open(&source).unwrap();
            db.execute_batch(sql).unwrap();
            drop(db);
            let digest = format!("{:x}", Sha256::digest(std::fs::read(&source).unwrap()));
            assert!(inspect(&source, Some(&digest)).is_err(), "accepted {sql}");
            assert!(
                restore(dir.path(), &source, &digest).is_err(),
                "restored {sql}"
            );
            assert_eq!(
                before,
                std::fs::read(&target).unwrap(),
                "changed target for {sql}"
            );
        }
    }
    #[tokio::test]
    async fn restore_refuses_running_invalid_digest_new_schema_and_preserves_original() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = Store::open(Some(source_dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = source.backup().await.unwrap();
        let export = source_dir.path().join("export.sqlite3");
        save_new(&export, &snapshot.bytes).unwrap();
        source.close().await.unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let bootstrap = DaemonConfig {
            instance_name: "before-restore".into(),
            ..Default::default()
        };
        let target = Store::open(Some(target_dir.path()), bootstrap).unwrap();
        assert!(matches!(
            restore(target_dir.path(), &export, &snapshot.metadata.sha256),
            Err(BackupError::Running)
        ));
        target.close().await.unwrap();
        assert!(matches!(
            restore(target_dir.path(), &export, &"0".repeat(64)),
            Err(BackupError::Invalid)
        ));
        let result = restore(target_dir.path(), &export, &snapshot.metadata.sha256).unwrap();
        assert!(result.restart_required);
        let old = readonly(&result.preserved_database.unwrap()).unwrap();
        let text: String = old
            .query_row(
                "SELECT document FROM config_revisions WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(text.contains("before-restore"));
        let reopened = Store::open(Some(target_dir.path()), DaemonConfig::default()).unwrap();
        assert_ne!(reopened.epoch, target.epoch);
        reopened.close().await.unwrap();
        let damaged = Connection::open(&export).unwrap();
        damaged
            .execute(
                "UPDATE config_revisions SET checksum=?1 WHERE id=1",
                ["0".repeat(64)],
            )
            .unwrap();
        drop(damaged);
        let damaged_sha = format!("{:x}", Sha256::digest(std::fs::read(&export).unwrap()));
        let before = std::fs::read(target_dir.path().join("hps.sqlite3")).unwrap();
        assert!(inspect(&export, Some(&damaged_sha)).is_err());
        assert!(restore(target_dir.path(), &export, &damaged_sha).is_err());
        assert_eq!(
            before,
            std::fs::read(target_dir.path().join("hps.sqlite3")).unwrap()
        );
        std::fs::write(&export, &snapshot.bytes).unwrap();
        let newer = Connection::open(&export).unwrap();
        newer
            .execute("INSERT INTO schema_migrations VALUES(999,0)", [])
            .unwrap();
        drop(newer);
        assert!(inspect(&export, None).is_err());
        let corrupt = source_dir.path().join("corrupt.sqlite3");
        save_new(&corrupt, b"not sqlite").unwrap();
        assert!(inspect(&corrupt, None).is_err());
        assert!(!std::fs::read_dir(target_dir.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".restore-")
        }));
    }
}
