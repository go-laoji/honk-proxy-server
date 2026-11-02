use super::*;
use sha2::{Digest, Sha256};
impl HpsClient {
    /// Explicit full database download. The snapshot includes upstream credentials.
    pub async fn database_backup(
        &self,
    ) -> Result<(hps_domain::BackupMetadata, Vec<u8>), ClientError> {
        let url = self
            .endpoint
            .join("api/v1/config/backup")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let mut response = self.http.post(url).send().await?;
        if !response.status().is_success() {
            return Self::decode::<serde_json::Value>(response)
                .await
                .map(|_| unreachable!());
        }
        let metadata: hps_domain::BackupMetadata = response
            .headers()
            .get("X-HPS-Backup")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| serde_json::from_str(v).ok())
            .ok_or(ClientError::BackupProtocol)?;
        if metadata.size_bytes > 64 * 1024 * 1024 || metadata.format_version != 1 {
            return Err(ClientError::BackupProtocol);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 64 * 1024 * 1024 {
                return Err(ClientError::BackupProtocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != metadata.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != metadata.sha256
        {
            return Err(ClientError::BackupProtocol);
        }
        Ok((metadata, bytes))
    }
}
