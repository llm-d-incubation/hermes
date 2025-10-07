use anyhow::Result;
use insta::{Settings, assert_snapshot};
use serde_json::Value;
use std::process::Command;

/// helper to run the command and capture output
fn run_command(args: &[&str], env_vars: &[(&str, &str)]) -> Result<(String, String, i32)> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--quiet") // suppress cargo build output
        .arg("--")
        .args(args);

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

/// snapshot test for CoreWeave self-test dry run
#[test]
fn test_coreweave_self_test_dry_run() -> Result<()> {
    let kubeconfig = std::env::var("COREWEAVE_KUBECONFIG")
        .expect("COREWEAVE_KUBECONFIG must be set for CoreWeave tests");

    let (stdout, stderr, _exit_code) = run_command(
        &["self-test", "--dry-run", "--namespace", "default"],
        &[("KUBECONFIG", &kubeconfig)],
    )?;

    // combine stdout and stderr since logging goes to stderr
    let combined_output = format!("{}{}", stderr, stdout);

    // extract the rendered manifest from output
    let manifest_start = combined_output.find("===").unwrap_or(0);
    let manifest_section = &combined_output[manifest_start..];

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        assert_snapshot!("coreweave_self_test_manifest", manifest_section);
    });

    Ok(())
}

/// snapshot test for OpenShift self-test dry run
#[test]
fn test_openshift_self_test_dry_run() -> Result<()> {
    let (stdout, stderr, _exit_code) = run_command(
        &["self-test", "--dry-run", "--namespace", "default"],
        &[], // uses default kubeconfig
    )?;

    // combine stdout and stderr since logging goes to stderr
    let combined_output = format!("{}{}", stderr, stdout);

    // extract the rendered manifest from output
    let manifest_start = combined_output.find("===").unwrap_or(0);
    let manifest_section = &combined_output[manifest_start..];

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        assert_snapshot!("openshift_self_test_manifest", manifest_section);
    });

    Ok(())
}

/// snapshot test for CoreWeave scan output (JSON format)
#[test]
fn test_coreweave_scan_json() -> Result<()> {
    let kubeconfig = std::env::var("COREWEAVE_KUBECONFIG")
        .expect("COREWEAVE_KUBECONFIG must be set for CoreWeave tests");

    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "json"],
        &[("KUBECONFIG", &kubeconfig)],
    )?;

    // parse JSON to validate it's well-formed before snapshotting
    let json: Value = serde_json::from_str(&stdout)?;
    let formatted = serde_json::to_string_pretty(&json)?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.add_redaction(".nodes[].name", "[node-name]");
    settings.add_redaction(".nodes[].platform_specific_labels", "[redacted]");
    settings.bind(|| {
        assert_snapshot!("coreweave_scan_json", formatted);
    });

    Ok(())
}

/// snapshot test for OpenShift scan output (JSON format)
#[test]
fn test_openshift_scan_json() -> Result<()> {
    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "json"],
        &[], // uses default kubeconfig
    )?;

    // parse JSON to validate it's well-formed before snapshotting
    let json: Value = serde_json::from_str(&stdout)?;
    let formatted = serde_json::to_string_pretty(&json)?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.add_redaction(".nodes[].name", "[node-name]");
    settings.add_redaction(".nodes[].platform_specific_labels", "[redacted]");
    settings.bind(|| {
        assert_snapshot!("openshift_scan_json", formatted);
    });

    Ok(())
}

/// snapshot test for CoreWeave scan with IB-only filter
#[test]
fn test_coreweave_scan_ib_only() -> Result<()> {
    let kubeconfig = std::env::var("COREWEAVE_KUBECONFIG")
        .expect("COREWEAVE_KUBECONFIG must be set for CoreWeave tests");

    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "json", "--ib-only"],
        &[("KUBECONFIG", &kubeconfig)],
    )?;

    let json: Value = serde_json::from_str(&stdout)?;
    let formatted = serde_json::to_string_pretty(&json)?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.add_redaction(".nodes[].name", "[node-name]");
    settings.bind(|| {
        assert_snapshot!("coreweave_scan_ib_only", formatted);
    });

    Ok(())
}

/// snapshot test for CoreWeave scan table format
#[test]
fn test_coreweave_scan_table() -> Result<()> {
    let kubeconfig = std::env::var("COREWEAVE_KUBECONFIG")
        .expect("COREWEAVE_KUBECONFIG must be set for CoreWeave tests");

    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "table"],
        &[("KUBECONFIG", &kubeconfig)],
    )?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        assert_snapshot!("coreweave_scan_table", stdout);
    });

    Ok(())
}

/// snapshot test for OpenShift scan table format
#[test]
fn test_openshift_scan_table() -> Result<()> {
    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "table"],
        &[], // uses default kubeconfig
    )?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.bind(|| {
        assert_snapshot!("openshift_scan_table", stdout);
    });

    Ok(())
}

/// snapshot test for CoreWeave detailed labels
#[test]
fn test_coreweave_scan_detailed() -> Result<()> {
    let kubeconfig = std::env::var("COREWEAVE_KUBECONFIG")
        .expect("COREWEAVE_KUBECONFIG must be set for CoreWeave tests");

    let (stdout, _stderr, _exit_code) = run_command(
        &["scan", "--format", "json", "--detailed-labels"],
        &[("KUBECONFIG", &kubeconfig)],
    )?;

    let json: Value = serde_json::from_str(&stdout)?;
    let formatted = serde_json::to_string_pretty(&json)?;

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path("../snapshots");
    settings.add_redaction(".nodes[].name", "[node-name]");
    settings.add_redaction(".nodes[].platform_specific_labels", "[redacted]");
    settings.bind(|| {
        assert_snapshot!("coreweave_scan_detailed", formatted);
    });

    Ok(())
}
