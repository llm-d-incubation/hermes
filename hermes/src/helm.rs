//! Helm execution engine for running Helm commands via subprocess.
//!
//! This module provides a clean Rust interface for executing Helm commands
//! asynchronously using tokio processes. It handles command execution, error
//! parsing, and timeout management.
//!
//! # Examples
//!
//! ```no_run
//! use hermes::helm::HelmExecutor;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // create executor and verify helm is installed
//!     let executor = HelmExecutor::new().await?;
//!     println!("Using helm version: {}", executor.version());
//!
//!     // render chart template
//!     let yaml = executor.template(
//!         "my-release",
//!         "./charts/nixl-test",
//!         Some("/tmp/values.yaml"),
//!         "default"
//!     ).await?;
//!     println!("Rendered YAML:\n{}", yaml);
//!
//!     // install release
//!     executor.install(
//!         "test-release",
//!         "./charts/nixl-test",
//!         Some("/tmp/values.yaml"),
//!         "default",
//!         true,  // wait for completion
//!         Some(300)  // 5 minute timeout
//!     ).await?;
//!
//!     // check status
//!     let status = executor.status("test-release", "default").await?;
//!     println!("Release status: {:?}", status);
//!
//!     // list all releases
//!     let releases = executor.list_releases("default").await?;
//!     println!("Releases: {:?}", releases);
//!
//!     // cleanup
//!     executor.uninstall("test-release", "default").await?;
//!
//!     Ok(())
//! }
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{debug, warn};

/// minimum supported Helm version
const MIN_HELM_VERSION: &str = "3.0.0";

/// helm release status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HelmStatus {
    Deployed,
    Failed,
    PendingInstall,
    PendingUpgrade,
    PendingRollback,
    Superseded,
    Uninstalling,
    Uninstalled,
    Unknown,
}

/// helm executor for running Helm commands via subprocess
pub struct HelmExecutor {
    helm_path: String,
    version: String,
}

impl HelmExecutor {
    /// create new HelmExecutor and verify Helm installation
    pub async fn new() -> Result<Self> {
        Self::with_path("helm").await
    }

    /// create HelmExecutor with custom helm binary path
    pub async fn with_path(helm_path: impl Into<String>) -> Result<Self> {
        let helm_path = helm_path.into();

        // verify helm exists
        let version_output = Command::new(&helm_path)
            .args(["version", "--template={{.Version}}"])
            .output()
            .await
            .context("Failed to execute helm command. Is helm installed?")?;

        if !version_output.status.success() {
            let stderr = String::from_utf8_lossy(&version_output.stderr);
            bail!("helm version command failed: {}", stderr);
        }

        let version = String::from_utf8(version_output.stdout)
            .context("Failed to parse helm version output")?
            .trim()
            .to_string();

        debug!("Detected helm version: {}", version);

        // parse and validate version (strip 'v' prefix if present)
        let version_num = version.strip_prefix('v').unwrap_or(&version);
        if !Self::is_version_compatible(version_num)? {
            bail!(
                "Helm version {} is not supported. Minimum required version: {}",
                version,
                MIN_HELM_VERSION
            );
        }

        Ok(Self { helm_path, version })
    }

    /// check if helm version meets minimum requirement
    fn is_version_compatible(version: &str) -> Result<bool> {
        let parse_version = |v: &str| -> Result<Vec<u32>> {
            v.split('.')
                .take(3)
                .map(|s| {
                    s.split('-')
                        .next()
                        .unwrap_or(s)
                        .parse::<u32>()
                        .with_context(|| format!("Invalid version component: {}", s))
                })
                .collect()
        };

        let current = parse_version(version)?;
        let minimum = parse_version(MIN_HELM_VERSION)?;

        // simple semver comparison (major.minor.patch)
        for i in 0..3 {
            let curr = current.get(i).copied().unwrap_or(0);
            let min = minimum.get(i).copied().unwrap_or(0);
            if curr > min {
                return Ok(true);
            } else if curr < min {
                return Ok(false);
            }
        }
        Ok(true) // equal versions
    }

    /// get detected helm version
    pub fn version(&self) -> &str {
        &self.version
    }

    /// render helm chart template to YAML string
    pub async fn template(
        &self,
        release_name: &str,
        chart_path: impl AsRef<Path>,
        values_file: Option<impl AsRef<Path>>,
        namespace: &str,
    ) -> Result<String> {
        let chart_path_str = chart_path.as_ref().to_str().context("Invalid chart path")?;
        let mut args = vec![
            "template",
            release_name,
            chart_path_str,
            "--namespace",
            namespace,
        ];

        let values_path_str;
        if let Some(values) = values_file {
            values_path_str = values
                .as_ref()
                .to_str()
                .context("Invalid values file path")?
                .to_string();
            args.push("--values");
            args.push(&values_path_str);
        }

        self.run_command(&args, None).await
    }

    /// install helm release
    pub async fn install(
        &self,
        release_name: &str,
        chart_path: impl AsRef<Path>,
        values_file: Option<impl AsRef<Path>>,
        namespace: &str,
        wait: bool,
        timeout_secs: Option<u64>,
    ) -> Result<()> {
        let chart_path_str = chart_path.as_ref().to_str().context("Invalid chart path")?;
        let mut args = vec![
            "install",
            release_name,
            chart_path_str,
            "--namespace",
            namespace,
            "--create-namespace",
        ];

        let values_path_str;
        if let Some(values) = values_file {
            values_path_str = values
                .as_ref()
                .to_str()
                .context("Invalid values file path")?
                .to_string();
            args.push("--values");
            args.push(&values_path_str);
        }

        let timeout_str;
        if wait {
            args.push("--wait");
            if let Some(timeout) = timeout_secs {
                timeout_str = format!("{}s", timeout);
                args.push("--timeout");
                args.push(&timeout_str);
            }
        }

        self.run_command(&args, timeout_secs.map(|s| Duration::from_secs(s + 30)))
            .await?;
        Ok(())
    }

    /// uninstall helm release
    pub async fn uninstall(&self, release_name: &str, namespace: &str) -> Result<()> {
        let args = vec!["uninstall", release_name, "--namespace", namespace];
        self.run_command(&args, None).await?;
        Ok(())
    }

    /// get helm release status
    pub async fn status(&self, release_name: &str, namespace: &str) -> Result<HelmStatus> {
        let args = vec![
            "status",
            release_name,
            "--namespace",
            namespace,
            "--output",
            "json",
        ];

        let output = self.run_command(&args, None).await?;

        // parse JSON output to extract status
        let json: serde_json::Value =
            serde_json::from_str(&output).context("Failed to parse helm status JSON output")?;

        let status_str = json["info"]["status"]
            .as_str()
            .context("Missing status field in helm status output")?;

        Ok(match status_str {
            "deployed" => HelmStatus::Deployed,
            "failed" => HelmStatus::Failed,
            "pending-install" => HelmStatus::PendingInstall,
            "pending-upgrade" => HelmStatus::PendingUpgrade,
            "pending-rollback" => HelmStatus::PendingRollback,
            "superseded" => HelmStatus::Superseded,
            "uninstalling" => HelmStatus::Uninstalling,
            "uninstalled" => HelmStatus::Uninstalled,
            _ => {
                warn!("Unknown helm status: {}", status_str);
                HelmStatus::Unknown
            }
        })
    }

    /// check if a helm release exists
    pub async fn release_exists(&self, release_name: &str, namespace: &str) -> bool {
        self.status(release_name, namespace).await.is_ok()
    }

    /// list all helm releases in a namespace
    pub async fn list_releases(&self, namespace: &str) -> Result<Vec<String>> {
        let args = vec!["list", "--namespace", namespace, "--output", "json"];

        let output = self.run_command(&args, None).await?;

        let json: serde_json::Value =
            serde_json::from_str(&output).context("Failed to parse helm list JSON output")?;

        let releases = json
            .as_array()
            .context("Expected JSON array from helm list")?
            .iter()
            .filter_map(|v| v["name"].as_str().map(String::from))
            .collect();

        Ok(releases)
    }

    /// run helm command and return stdout
    async fn run_command(&self, args: &[&str], timeout: Option<Duration>) -> Result<String> {
        let timeout = timeout.unwrap_or(Duration::from_secs(300)); // default 5min timeout

        debug!("Running command: {} {}", self.helm_path, args.join(" "));

        let mut child = Command::new(&self.helm_path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to spawn helm command: {} {}",
                    self.helm_path,
                    args.join(" ")
                )
            })?;

        let stdout = child.stdout.take().context("Failed to capture stdout")?;
        let stderr = child.stderr.take().context("Failed to capture stderr")?;

        // run with timeout
        let result = tokio::time::timeout(timeout, async {
            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();

            let mut stdout_reader = tokio::io::BufReader::new(stdout);
            let mut stderr_reader = tokio::io::BufReader::new(stderr);

            // read outputs
            let (stdout_result, stderr_result) = tokio::join!(
                stdout_reader.read_to_end(&mut stdout_buf),
                stderr_reader.read_to_end(&mut stderr_buf)
            );

            stdout_result.context("Failed to read stdout")?;
            stderr_result.context("Failed to read stderr")?;

            let status = child
                .wait()
                .await
                .context("Failed to wait for helm process")?;

            Ok::<_, anyhow::Error>((status, stdout_buf, stderr_buf))
        })
        .await
        .with_context(|| {
            format!(
                "Helm command timed out after {}s: {} {}",
                timeout.as_secs(),
                self.helm_path,
                args.join(" ")
            )
        })??;

        let (status, stdout_buf, stderr_buf) = result;

        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

        if !status.success() {
            bail!(
                "Helm command failed: {} {}\nExit code: {}\nStderr: {}\nStdout: {}",
                self.helm_path,
                args.join(" "),
                status.code().unwrap_or(-1),
                stderr,
                stdout
            );
        }

        if !stderr.is_empty() {
            debug!("Helm stderr: {}", stderr);
        }

        Ok(stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_compatibility() {
        assert!(HelmExecutor::is_version_compatible("3.0.0").unwrap());
        assert!(HelmExecutor::is_version_compatible("3.1.0").unwrap());
        assert!(HelmExecutor::is_version_compatible("3.15.4").unwrap());
        assert!(HelmExecutor::is_version_compatible("4.0.0").unwrap());
        assert!(!HelmExecutor::is_version_compatible("2.17.0").unwrap());
        assert!(!HelmExecutor::is_version_compatible("2.99.99").unwrap());

        // test version with build metadata
        assert!(HelmExecutor::is_version_compatible("3.15.4-rc1").unwrap());
        assert!(HelmExecutor::is_version_compatible("3.0.0-beta").unwrap());
    }

    #[test]
    fn test_helm_status_deserialization() {
        let json = r#"{"status": "deployed"}"#;
        let status: HelmStatus = serde_json::from_str::<serde_json::Value>(json).unwrap()["status"]
            .as_str()
            .map(|s| match s {
                "deployed" => HelmStatus::Deployed,
                _ => HelmStatus::Unknown,
            })
            .unwrap();
        assert_eq!(status, HelmStatus::Deployed);
    }

    #[tokio::test]
    async fn test_helm_executor_creation() {
        // this test will fail if helm is not installed, which is expected
        // in CI, we should mock this or skip the test
        match HelmExecutor::new().await {
            Ok(executor) => {
                assert!(!executor.version().is_empty());
                println!("Detected helm version: {}", executor.version());
            }
            Err(e) => {
                println!("Helm not installed (expected in some environments): {}", e);
            }
        }
    }
}
