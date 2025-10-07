# Hermes

Kubernetes cluster analyzer for RDMA-capable GPU infrastructure. Scans clusters to detect RDMA networking capabilities, GPU topology, and intelligently selects optimal node pairs for high-speed interconnect testing.

Supports CoreWeave, GKE, OpenShift, and generic Kubernetes environments.

## Installation

### From Release Binary

Download the latest release for your platform:

```bash
# macOS (Apple Silicon)
curl -LO https://github.com/llm-d-incubation/hermes/releases/download/v0.1.0/hermes-aarch64-apple-darwin.tar.gz
tar xzf hermes-aarch64-apple-darwin.tar.gz
sudo mv hermes /usr/local/bin/

# macOS (Intel)
curl -LO https://github.com/llm-d-incubation/hermes/releases/download/v0.1.0/hermes-x86_64-apple-darwin.tar.gz
tar xzf hermes-x86_64-apple-darwin.tar.gz
sudo mv hermes /usr/local/bin/

# Linux (x86_64)
curl -LO https://github.com/llm-d-incubation/hermes/releases/download/v0.1.0/hermes-x86_64-unknown-linux-gnu.tar.gz
tar xzf hermes-x86_64-unknown-linux-gnu.tar.gz
sudo mv hermes /usr/local/bin/
```

### From Source

```bash
cargo install --path .
```

## Quick Start

```bash
# scan cluster
hermes scan

# filter RDMA-capable nodes
hermes scan --ib-only

# preview RDMA test manifests
hermes self-test --dry-run

# run RDMA self-test
hermes self-test --namespace default
```

## Platform Examples

```bash
# CoreWeave
KUBECONFIG=~/path/to/cwconfig hermes scan

# GKE
gcloud container clusters get-credentials CLUSTER_NAME && hermes scan

# OpenShift (with proxy)
HTTPS_PROXY=http://proxy-ip:port hermes scan
```

## Self-Test Framework

Automatically deploys RDMA workloads on intelligently-selected node pairs:

```bash
# preview what will be deployed
hermes self-test --dry-run

# run UCX-based data transfer test
hermes self-test --namespace default

# OpenShift RoCE (auto-detects SR-IOV network or use --sriov-network)
hermes self-test --namespace test-ns

# keep resources after test
hermes self-test --no-cleanup
```

**How it works**: Scans cluster → selects optimal node pair (same fabric/zone) → renders test manifests → deploys jobs → monitors completion → cleanup

**Available workloads**: `nixl-transfer-test` (default), `deepgemm-minimal-test`

## Output Formats

```bash
hermes scan --format json    # JSON output
hermes scan --format table   # table view (default)
hermes scan --save-to report.json
```

## License

MIT
