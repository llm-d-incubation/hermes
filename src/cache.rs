use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::{debug, info};

use crate::models::ClusterReport;

const CACHE_VERSION: u8 = 1;
const DEFAULT_CACHE_TTL_HOURS: i64 = 24;

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedScan {
    /// cache format version for future compatibility
    version: u8,
    /// when this scan was performed
    timestamp: DateTime<Utc>,
    /// kubeconfig context identifier (hash)
    context_key: String,
    /// the actual scan data
    report: ClusterReport,
}

impl CachedScan {
    pub fn new(context_key: String, report: ClusterReport) -> Self {
        Self {
            version: CACHE_VERSION,
            timestamp: Utc::now(),
            context_key,
            report,
        }
    }

    pub fn is_valid(&self, context_key: &str, ttl_hours: Option<i64>) -> bool {
        // check version compatibility
        if self.version != CACHE_VERSION {
            debug!(
                "Cache version mismatch: {} != {}",
                self.version, CACHE_VERSION
            );
            return false;
        }

        // check context match
        if self.context_key != context_key {
            debug!(
                "Cache context mismatch: {} != {}",
                self.context_key, context_key
            );
            return false;
        }

        // check if cache has expired
        let ttl = Duration::hours(ttl_hours.unwrap_or(DEFAULT_CACHE_TTL_HOURS));
        let age = Utc::now() - self.timestamp;
        if age > ttl {
            debug!("Cache expired: age={:?}, ttl={:?}", age, ttl);
            return false;
        }

        true
    }

    pub fn report(&self) -> &ClusterReport {
        &self.report
    }
}

/// manages XDG-compliant cache storage for cluster scans
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to determine XDG cache directory")?
            .join("hermes");

        // ensure cache directory exists
        std::fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;

        Ok(Self { cache_dir })
    }

    /// generates a cache key from kubeconfig context information
    /// includes: cluster server URL, namespace, and context name
    pub fn generate_context_key(config: &kube::Config) -> String {
        let mut hasher = Sha256::new();

        // hash cluster server URL (primary identifier)
        hasher.update(config.cluster_url.to_string().as_bytes());

        // hash default namespace
        hasher.update(config.default_namespace.as_bytes());

        // note: kube::Config doesn't directly expose context name,
        // but cluster_url + namespace should be unique enough

        let hash = hasher.finalize();
        format!("{:x}", hash)
    }

    /// returns the cache file path for a given context key
    fn cache_file_path(&self, context_key: &str) -> PathBuf {
        self.cache_dir.join(format!("scan-{}.json", context_key))
    }

    /// loads cached scan data if valid
    pub fn load(&self, context_key: &str, ttl_hours: Option<i64>) -> Result<Option<ClusterReport>> {
        let cache_file = self.cache_file_path(context_key);

        if !cache_file.exists() {
            debug!("Cache file not found: {:?}", cache_file);
            return Ok(None);
        }

        let cache_data =
            std::fs::read_to_string(&cache_file).context("Failed to read cache file")?;

        let cached_scan: CachedScan =
            serde_json::from_str(&cache_data).context("Failed to parse cache file")?;

        if !cached_scan.is_valid(context_key, ttl_hours) {
            info!("Cache invalid or expired, will perform fresh scan");
            // clean up invalid cache file
            let _ = std::fs::remove_file(&cache_file);
            return Ok(None);
        }

        let age = Utc::now() - cached_scan.timestamp;
        info!(
            "Using cached scan from {} ago ({})",
            humanize_duration(age),
            cached_scan.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );

        Ok(Some(cached_scan.report))
    }

    /// saves scan data to cache
    pub fn save(&self, context_key: &str, report: &ClusterReport) -> Result<()> {
        let cache_file = self.cache_file_path(context_key);
        let cached_scan = CachedScan::new(context_key.to_string(), report.clone());

        let cache_data =
            serde_json::to_string_pretty(&cached_scan).context("Failed to serialize cache data")?;

        std::fs::write(&cache_file, cache_data).context("Failed to write cache file")?;

        info!("Cached scan data to: {:?}", cache_file);
        Ok(())
    }

    /// clears all cached scans
    pub fn clear_all(&self) -> Result<()> {
        if !self.cache_dir.exists() {
            return Ok(());
        }

        let mut removed_count = 0;
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                std::fs::remove_file(&path)?;
                removed_count += 1;
            }
        }

        info!("Cleared {} cached scan(s)", removed_count);
        Ok(())
    }

    /// removes expired cache entries
    pub fn prune_expired(&self, ttl_hours: Option<i64>) -> Result<()> {
        if !self.cache_dir.exists() {
            return Ok(());
        }

        let ttl = Duration::hours(ttl_hours.unwrap_or(DEFAULT_CACHE_TTL_HOURS));
        let mut pruned_count = 0;

        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && let Ok(cache_data) = std::fs::read_to_string(&path)
                && let Ok(cached_scan) = serde_json::from_str::<CachedScan>(&cache_data)
            {
                let age = Utc::now() - cached_scan.timestamp;
                if age > ttl {
                    std::fs::remove_file(&path)?;
                    pruned_count += 1;
                    debug!("Pruned expired cache: {:?} (age: {:?})", path, age);
                }
            }
        }

        if pruned_count > 0 {
            info!("Pruned {} expired cache file(s)", pruned_count);
        }

        Ok(())
    }
}

/// converts chrono duration to human-readable format
fn humanize_duration(duration: Duration) -> String {
    let hours = duration.num_hours();
    let minutes = duration.num_minutes() % 60;

    if hours > 0 {
        if minutes > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}h", hours)
        }
    } else if minutes > 0 {
        format!("{}m", minutes)
    } else {
        let seconds = duration.num_seconds();
        format!("{}s", seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_humanize_duration() {
        assert_eq!(humanize_duration(Duration::seconds(30)), "30s");
        assert_eq!(humanize_duration(Duration::minutes(5)), "5m");
        assert_eq!(humanize_duration(Duration::hours(2)), "2h");
        assert_eq!(
            humanize_duration(Duration::hours(2) + Duration::minutes(30)),
            "2h 30m"
        );
    }
}
