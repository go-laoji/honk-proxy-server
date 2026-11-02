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
    #[error("backup exceeded the 30 second snapshot deadline")]
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
    let version: i64 =
        connection.query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(BackupError::Invalid);
    }
    let reference = Connection::open_in_memory()?;
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
    let mut statement = reference.prepare(
        "SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
    )?;
    for item in statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, expected) = item?;
        let actual: Option<String> = connection
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()?;
        if actual.as_deref() != Some(expected.as_str()) {
            return Err(BackupError::Invalid);
        }
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
    Ok(version)
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
pub fn inspect(
    path: &Path,
    expected_sha256: Option<&str>,
) -> Result<hps_domain::BackupMetadata, BackupError> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_BACKUP_BYTES {
        return Err(BackupError::Invalid);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    Read::by_ref(&mut file)
        .take(MAX_BACKUP_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if bytes.len() as u64 != size || expected_sha256.is_some_and(|expected| expected != digest) {
        return Err(BackupError::Invalid);
    }
    let connection = readonly(path)?;
    let journal: String = connection.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if journal != "delete" {
        return Err(BackupError::Invalid);
    }
    let schema_version = validate(&connection)?;
    Ok(hps_domain::BackupMetadata {
        format_version: 1,
        schema_version,
        size_bytes: size,
        sha256: digest,
        verified_at_unix: unix_now(),
        contains_credentials: true,
        restore_requires_stopped_daemon: true,
    })
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
    let original = directory.join("hps.sqlite3");
    if source.canonicalize()? == original {
        return Err(BackupError::Invalid);
    }
    // Validate the caller's digest before touching the target. Recheck the source after copying.
    inspect(source, Some(expected_sha256))?;
    let temporary = Temporary(directory.join(format!(".restore-{}.sqlite3", uuid::Uuid::new_v4())));
    copy(&readonly(source)?, &temporary.0)?;
    inspect(source, Some(expected_sha256))?;
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
    File::open(&directory)
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
            let _permit = permit;
            let temporary =
                Temporary(path.with_file_name(format!(".backup-{}.sqlite3", uuid::Uuid::new_v4())));
            copy(&readonly(&path)?, &temporary.0)?;
            let metadata = inspect(&temporary.0, None)?;
            let bytes = std::fs::read(&temporary.0)?;
            Ok(Snapshot { metadata, bytes })
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
        writer.execute_batch("CREATE TABLE backup_pair(a INTEGER,b INTEGER CHECK(a=b)); INSERT INTO backup_pair VALUES(0,0);CREATE TABLE backup_payload(data BLOB);INSERT INTO backup_payload VALUES(zeroblob(2097152))").unwrap();
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let signal = running.clone();
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count = progress.clone();
        let worker = std::thread::spawn(move || {
            let mut n = 0;
            while signal.load(std::sync::atomic::Ordering::Relaxed) {
                writer
                    .execute("UPDATE backup_pair SET a=?1,b=?1", [n])
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
            .query_row("SELECT a,b FROM backup_pair", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
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
    async fn restore_refuses_running_invalid_digest_new_schema_and_preserves_original() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = Store::open(Some(source_dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = source.backup().await.unwrap();
        let export = source_dir.path().join("export.sqlite3");
        save_new(&export, &snapshot.bytes).unwrap();
        source.close().await.unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let mut bootstrap = DaemonConfig::default();
        bootstrap.instance_name = "before-restore".into();
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
