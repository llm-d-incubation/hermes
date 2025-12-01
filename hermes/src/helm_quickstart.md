# Helm Executor Quick Reference

## Import
```rust
use crate::helm::{HelmExecutor, HelmStatus};
```

## Initialization
```rust
// Auto-detect helm in PATH
let helm = HelmExecutor::new().await?;

// Or use custom path
let helm = HelmExecutor::with_path("/usr/local/bin/helm").await?;

// Get version
println!("Using Helm {}", helm.version());
```

## Template Rendering (Dry-Run)
```rust
let yaml = helm.template(
    "my-release",                              // release name
    "./charts/workloads/nixl-transfer-test",   // chart path
    Some("/tmp/values.yaml"),                  // optional values file
    "default"                                  // namespace
).await?;

println!("{}", yaml);
```

## Installation
```rust
helm.install(
    "test-release",                            // release name
    "./charts/workloads/nixl-transfer-test",   // chart path
    Some("/tmp/values.yaml"),                  // optional values file
    "default",                                 // namespace
    true,                                      // wait for completion
    Some(300)                                  // timeout in seconds (optional)
).await?;
```

## Status Checking
```rust
// Get detailed status
let status = helm.status("test-release", "default").await?;
match status {
    HelmStatus::Deployed => println!("Success!"),
    HelmStatus::Failed => println!("Deployment failed"),
    _ => println!("Status: {:?}", status),
}

// Quick existence check
if helm.release_exists("test-release", "default").await {
    println!("Release exists");
}
```

## Listing Releases
```rust
let releases = helm.list_releases("default").await?;
for release in releases {
    println!("Found release: {}", release);
}
```

## Cleanup
```rust
helm.uninstall("test-release", "default").await?;
```

## Error Handling
```rust
match helm.install(...).await {
    Ok(_) => println!("Deployed successfully"),
    Err(e) => {
        eprintln!("Deployment failed: {}", e);
        // Error includes: command, exit code, stderr, stdout
    }
}
```

## Complete Example
```rust
use anyhow::Result;
use crate::helm::{HelmExecutor, HelmStatus};

async fn deploy_test() -> Result<()> {
    let helm = HelmExecutor::new().await?;

    // Deploy
    helm.install(
        "my-test",
        "./charts/workloads/nixl-transfer-test",
        Some("./values.yaml"),
        "default",
        true,
        Some(300)
    ).await?;

    // Verify
    let status = helm.status("my-test", "default").await?;
    assert_eq!(status, HelmStatus::Deployed);

    // Cleanup
    helm.uninstall("my-test", "default").await?;

    Ok(())
}
```
