use anyhow::Result;
use hermes::helm::HelmExecutor;

#[tokio::main]
async fn main() -> Result<()> {
    // initialize tracing for debug output
    tracing_subscriber::fmt()
        .with_env_filter("hermes=debug")
        .init();

    println!("=== Helm Executor Demo ===\n");

    // create executor and verify helm installation
    println!("1. Checking Helm installation...");
    let executor = HelmExecutor::new().await?;
    println!("   ✓ Helm version detected: {}\n", executor.version());

    // demonstrate template rendering (dry-run, no cluster needed)
    println!("2. Testing template rendering...");
    println!("   Note: This requires a Helm chart to exist at the specified path");
    println!("   Skipping actual template test (would need real chart)\n");

    // demonstrate release listing
    println!("3. Listing releases in default namespace...");
    match executor.list_releases("default").await {
        Ok(releases) => {
            if releases.is_empty() {
                println!("   No releases found in default namespace");
            } else {
                println!("   Found {} release(s):", releases.len());
                for release in releases {
                    println!("     - {}", release);
                }
            }
        }
        Err(e) => {
            println!("   Could not list releases: {}", e);
            println!("   (This is expected if kubectl is not configured)");
        }
    }

    println!("\n=== Demo Complete ===");
    println!("\nNext steps for integration:");
    println!("  1. Use executor.template() to render Helm charts");
    println!("  2. Use executor.install() to deploy releases");
    println!("  3. Use executor.status() to check deployment status");
    println!("  4. Use executor.uninstall() to cleanup");

    Ok(())
}
