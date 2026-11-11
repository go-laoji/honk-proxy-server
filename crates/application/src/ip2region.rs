use crate::{ApplicationError, Supervisor};
use hps_domain::{
    Ip2RegionSource, Ip2RegionStatus, Ip2RegionUpdate, NodeLocation, NodeLocationSource,
};
use ip2region::{CachePolicy, Searcher};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, RwLockReadGuard},
    time::Duration,
};
use tempfile::{Builder, NamedTempFile, TempDir};

const METADATA: &str =
    "https://api.github.com/repos/lionsoul2014/ip2region/contents/data?ref=master";
const RAW: &str = "https://raw.githubusercontent.com/lionsoul2014/ip2region/master/data";
const V4_FILE: &str = "ip2region_v4.xdb";
const V6_FILE: &str = "ip2region_v6.xdb";
const INDEX_START: usize = 256 + 256 * 256 * 8;
const MAX_XDB_BYTES: usize = 128 * 1024 * 1024;
type Seed = (&'static [u8], &'static [u8], &'static [u8]);
const BUNDLED: Option<Seed> = include!(concat!(env!("OUT_DIR"), "/ip2region_seed.rs"));

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("could not download ip2region data from GitHub")]
    Download(#[from] reqwest::Error),
    #[error(
        "ip2region database data is incomplete, changed during download or uses an unsupported format"
    )]
    InvalidData,
    #[error("could not store the ip2region database in the service data directory")]
    Storage(#[from] std::io::Error),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Blob {
    sha: String,
    size: usize,
}
impl Blob {
    fn valid(&self) -> bool {
        self.sha.len() == 40
            && self.sha.bytes().all(|b| b.is_ascii_hexdigit())
            && (INDEX_START + 1..=MAX_XDB_BYTES).contains(&self.size)
    }

    fn matches(&self, bytes: &[u8]) -> bool {
        if !self.valid() || self.size != bytes.len() {
            return false;
        }
        let mut hash = Sha1::new();
        hash.update(format!("blob {}\0", self.size));
        hash.update(bytes);
        format!("{:x}", hash.finalize()) == self.sha
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    ipv4: Blob,
    ipv6: Blob,
}
impl Manifest {
    fn version(&self) -> String {
        format!("{}/{}", &self.ipv4.sha[..12], &self.ipv6.sha[..12])
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, UpdateError> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|_| UpdateError::InvalidData)?;
        if !manifest.ipv4.valid() || !manifest.ipv6.valid() {
            return Err(UpdateError::InvalidData);
        }
        Ok(manifest)
    }
}

pub fn bundled_version() -> Option<String> {
    BUNDLED.and_then(|(meta, _, _)| Manifest::from_bytes(meta).ok().map(|m| m.version()))
}

pub(super) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && !ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            let [a, b, ..] = ip.segments();
            (a & 0xe000) == 0x2000 && (a, b) != (0x2001, 0x0db8)
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Active {
    directory: String,
    source: Ip2RegionSource,
}

pub(super) struct Database {
    manifest: Manifest,
    source: Ip2RegionSource,
    directory: PathBuf,
    ipv4: Searcher,
    ipv6: Searcher,
    temporary: Option<TempDir>,
}
impl Database {
    fn open(
        directory: PathBuf,
        manifest: Manifest,
        source: Ip2RegionSource,
    ) -> Result<Self, UpdateError> {
        let ipv4 = searcher(&directory.join(V4_FILE), &manifest.ipv4, 4)?;
        let ipv6 = searcher(&directory.join(V6_FILE), &manifest.ipv6, 6)?;
        Ok(Self {
            manifest,
            source,
            directory,
            ipv4,
            ipv6,
            temporary: None,
        })
    }

    pub(super) fn status(&self) -> Ip2RegionStatus {
        Ip2RegionStatus {
            version: Some(self.manifest.version()),
            source: Some(self.source),
        }
    }

    pub(super) fn locate(&self, ip: IpAddr, source: NodeLocationSource) -> Option<NodeLocation> {
        let record = match ip {
            IpAddr::V4(ip) => self.ipv4.search(ip),
            IpAddr::V6(ip) => self.ipv6.search(ip),
        }
        .ok()?;
        let mut fields = record
            .split('|')
            .map(str::trim)
            .map(|s| (!s.is_empty() && s != "0").then(|| s.to_owned()));
        let country = fields.next().flatten()?;
        let province = fields.next().flatten();
        let city = fields.next().flatten();
        let isp = fields.next().flatten();
        let country_code = fields
            .next()
            .flatten()
            .filter(|code| code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()));
        // The upstream DB also describes reserved ranges such as private and loopback addresses.
        // Geographic rows have a country and can have a province or a city even without an ISO code.
        if country_code.is_none() && province.is_none() && city.is_none() {
            return None;
        }
        Some(NodeLocation {
            ip,
            source,
            country: Some(country),
            province,
            city,
            isp,
            country_code,
        })
    }
}

fn searcher(path: &Path, blob: &Blob, family: u8) -> Result<Searcher, UpdateError> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_XDB_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if !blob.matches(&bytes) || bytes[..4] != [3, 0, 1, 0] || bytes[16..20] != [family, 0, 4, 0] {
        return Err(UpdateError::InvalidData);
    }
    let ptr = |bytes: &[u8]| u32::from_le_bytes(bytes.try_into().unwrap()) as usize;
    let first = ptr(&bytes[8..12]);
    let last = ptr(&bytes[12..16]);
    let stride = if family == 4 { 14 } else { 38 };
    if first < INDEX_START
        || last < first
        || last > bytes.len() - stride
        || (last - first) % stride != 0
    {
        return Err(UpdateError::InvalidData);
    }
    for vector in bytes[256..INDEX_START].as_chunks::<8>().0 {
        let start = ptr(&vector[..4]);
        let end = ptr(&vector[4..]);
        if (start, end) != (0, 0)
            && (start < first
                || start > last
                || end < start
                || end > last + stride
                || (start - first) % stride != 0
                || (end - first) % stride != 0)
        {
            return Err(UpdateError::InvalidData);
        }
    }
    for segment in bytes[first..last + stride].chunks_exact(stride) {
        let length =
            u16::from_le_bytes(segment[stride - 6..stride - 4].try_into().unwrap()) as usize;
        let start = ptr(&segment[stride - 4..]);
        if start < INDEX_START || start > first || length > first - start {
            return Err(UpdateError::InvalidData);
        }
    }
    drop(bytes);
    let path = path.to_str().ok_or(UpdateError::InvalidData)?;
    let searcher = Searcher::new(path.to_owned(), CachePolicy::VectorIndex)
        .map_err(|_| UpdateError::InvalidData)?;
    searcher
        .vector_index()
        .map_err(|_| UpdateError::InvalidData)?;
    Ok(searcher)
}

pub(super) struct Ip2Region {
    root: Option<PathBuf>,
    database: RwLock<Option<Database>>,
    updating: Arc<tokio::sync::Mutex<()>>,
}
impl Ip2Region {
    pub(super) fn new(root: Option<PathBuf>) -> Self {
        let database = root.as_deref().and_then(|root| match Self::load(root) {
            Ok(database) => database,
            Err(error) => {
                tracing::warn!(%error, "could not open persisted ip2region data");
                None
            }
        });
        let state = Self {
            root,
            database: RwLock::new(database),
            updating: Default::default(),
        };
        if let Some((meta, v4, v6)) = BUNDLED
            && let Err(error) = state.seed(meta, v4, v6)
        {
            tracing::warn!(%error, "could not open bundled ip2region data");
        }
        state
    }

    pub(super) fn read(&self) -> RwLockReadGuard<'_, Option<Database>> {
        self.database.read().unwrap()
    }

    fn load(root: &Path) -> Result<Option<Database>, UpdateError> {
        let bytes = match read_small(&root.join("active.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let active: Active =
            serde_json::from_slice(&bytes).map_err(|_| UpdateError::InvalidData)?;
        if !active
            .directory
            .strip_prefix("snapshot-")
            .is_some_and(|suffix| {
                suffix.len() == 6 && suffix.bytes().all(|c| c.is_ascii_alphanumeric())
            })
        {
            return Err(UpdateError::InvalidData);
        }
        let directory = root.join(&active.directory);
        let manifest = Manifest::from_bytes(&read_small(&directory.join("manifest.json"))?)?;
        Database::open(directory, manifest, active.source).map(Some)
    }

    fn seed(&self, meta: &[u8], v4: &[u8], v6: &[u8]) -> Result<(), UpdateError> {
        let manifest = Manifest::from_bytes(meta)?;
        if self
            .read()
            .as_ref()
            .is_some_and(|db| db.source == Ip2RegionSource::Updated || db.manifest == manifest)
        {
            return Ok(());
        }
        self.install(manifest, v4, v6, Ip2RegionSource::Bundled)?;
        Ok(())
    }

    fn install(
        &self,
        manifest: Manifest,
        v4: &[u8],
        v6: &[u8],
        source: Ip2RegionSource,
    ) -> Result<Ip2RegionStatus, UpdateError> {
        let mut builder = Builder::new();
        builder.prefix("snapshot-").rand_bytes(6);
        let stage = match &self.root {
            Some(root) => {
                fs::create_dir_all(root)?;
                builder.tempdir_in(root)?
            }
            None => builder.tempdir()?,
        };
        for (name, data) in [
            (V4_FILE, v4),
            (V6_FILE, v6),
            ("manifest.json", &encode(&manifest)?),
        ] {
            let mut file = File::create_new(stage.path().join(name))?;
            file.write_all(data)?;
            file.sync_all()?;
        }
        let mut database = Database::open(stage.path().to_owned(), manifest, source)?;
        let status = database.status();
        let mut current = self.database.write().unwrap();
        let durable =
            if let Some(root) = &self.root {
                #[cfg(unix)]
                File::open(stage.path())?.sync_all()?;
                let directory = stage
                    .path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let mut pointer = NamedTempFile::new_in(root)?;
                pointer.write_all(&encode(&Active { directory, source })?)?;
                pointer.as_file().sync_all()?;
                pointer
                    .persist(root.join("active.json"))
                    .map_err(|error| error.error)?;
                let _ = stage.keep();
                #[cfg(unix)]
                {
                    File::open(root).and_then(|f| f.sync_all()).inspect_err(|error| {
                    tracing::warn!(%error, "could not sync the published ip2region directory");
                }).is_ok()
                }
                #[cfg(not(unix))]
                true
            } else {
                database.temporary = Some(stage);
                true
            };
        let previous = current.replace(database);
        drop(current);
        if durable
            && let Some(previous) = previous
            && previous.temporary.is_none()
            && let Err(error) = fs::remove_dir_all(previous.directory)
        {
            tracing::warn!(%error, "could not remove the previous ip2region snapshot");
        }
        Ok(status)
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, UpdateError> {
    serde_json::to_vec(value).map_err(|_| UpdateError::InvalidData)
}

fn read_small(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    #[serde(flatten)]
    blob: Blob,
}

async fn download(request: reqwest::RequestBuilder, limit: usize) -> Result<Vec<u8>, UpdateError> {
    let mut response = request.send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(UpdateError::InvalidData);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit - bytes.len() {
            return Err(UpdateError::InvalidData);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn upstream(client: &reqwest::Client, url: &str) -> Result<Manifest, UpdateError> {
    let data = download(client.get(url).timeout(Duration::from_secs(10)), 64 * 1024).await?;
    let entries: Vec<Entry> =
        serde_json::from_slice(&data).map_err(|_| UpdateError::InvalidData)?;
    let blob = |name| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.blob.clone())
            .filter(Blob::valid)
            .ok_or(UpdateError::InvalidData)
    };
    Ok(Manifest {
        ipv4: blob(V4_FILE)?,
        ipv6: blob(V6_FILE)?,
    })
}

impl Supervisor {
    pub async fn update_ip2region(self: &Arc<Self>) -> Result<Ip2RegionUpdate, ApplicationError> {
        let client = reqwest::Client::builder()
            .user_agent("honk-proxy-server")
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(240))
            .build()
            .map_err(UpdateError::from)?;
        self.update_ip2region_from(client, METADATA, RAW).await
    }

    async fn update_ip2region_from(
        self: &Arc<Self>,
        client: reqwest::Client,
        metadata: &str,
        raw: &str,
    ) -> Result<Ip2RegionUpdate, ApplicationError> {
        let permit = self
            .ip2region
            .updating
            .clone()
            .try_lock_owned()
            .map_err(|_| ApplicationError::Ip2RegionBusy)?;
        let metadata = metadata.to_owned();
        let raw = raw.to_owned();
        self.tracked(move |service| async move {
            let manifest = tokio::select! {
                _ = service.provider_cancel.cancelled() => return Err(hps_engine::runtime::EngineError::ShuttingDown.into()),
                result = upstream(&client, &metadata) => result?,
            };
            if let Some(database) = service
                .ip2region
                .read()
                .as_ref()
                .filter(|db| db.manifest == manifest)
            {
                return Ok(Ip2RegionUpdate {
                    database: database.status(),
                    updated: false,
                });
            }
            let files = async {
                tokio::try_join!(
                    download(client.get(format!("{raw}/{V4_FILE}")), manifest.ipv4.size),
                    download(client.get(format!("{raw}/{V6_FILE}")), manifest.ipv6.size),
                )
            };
            let (v4, v6) = tokio::select! {
                _ = service.provider_cancel.cancelled() => return Err(hps_engine::runtime::EngineError::ShuttingDown.into()),
                result = files => result?,
            };
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let database =
                    service
                        .ip2region
                        .install(manifest, &v4, &v6, Ip2RegionSource::Updated)?;
                Ok(Ip2RegionUpdate {
                    database,
                    updated: true,
                })
            })
            .await
            .map_err(|_| ApplicationError::Worker)?
        })
        .await
    }
}

#[cfg(test)]
mod tests;
