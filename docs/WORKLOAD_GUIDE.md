# Workload Test Guide

Quick primer on adding new tests to Hermes.

## Directory Structure

```
manifests/
├── 01_nixl_transfer/
│   ├── manifest.yaml.j2        # k8s manifest template
│   ├── target-entrypoint.sh    # embedded at build time
│   └── initiator-entrypoint.sh
├── 06_deepep_low_latency/
│   ├── manifest.yaml.j2
│   ├── master-entrypoint.sh
│   └── worker-entrypoint.sh
```

Put your `manifest.yaml.j2` template in the workload directory. Other files (scripts, configs) get embedded automatically.

## File Embedding

`build.rs` reads non-`.j2` files from workload directories, base64-encodes them, and exposes them via the `configmap_files` template variable.

**In your workload (`src/workloads/your_test.rs`):**

```rust
fn render_manifest(&self, test_id: &str, node_pair: &NodePair,
                   config: &SelfTestConfig, rdma_info: &RdmaInfo) -> Result<String> {
    let context = TemplateContext::new(test_id, node_pair, config, rdma_info)
        .with_embedded_files("06_deepep_low_latency");

    let template_str = include_str!("../../manifests/06_deepep_low_latency/manifest.yaml.j2");
    let mut env = Environment::new();
    env.add_template("my_test", template_str)?;
    env.get_template("my_test")?.render(&context)
}
```

**In your template (`manifest.yaml.j2`):**

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: my-test-scripts-{{ test_id }}
binaryData:
{%- for key, value in configmap_files | items %}
  {{ key }}: {{ value }}
{%- endfor %}
---
apiVersion: batch/v1
kind: Job
spec:
  template:
    spec:
      containers:
      - name: worker
        volumeMounts:
        - name: scripts
          mountPath: /opt/scripts
      volumes:
      - name: scripts
        configMap:
          name: my-test-scripts-{{ test_id }}
          defaultMode: 493  # 0755
```

### What gets embedded

- Scripts (`.sh`, `.py`, etc.)
- Configs (`.yaml`, `.json`, `.toml`, etc.)
- Binaries (up to 10MB)
- Not `.j2` files (use `include_str!` for those)

Max file size: 10MB. Build fails if exceeded.

## Template Variables

```jinja2
{{ test_id }}              # 8-char test ID
{{ server_node.name }}     # server node name
{{ client_node.name }}     # client node name
{{ server_node.rdma_device }}
{{ client_node.rdma_device }}
{{ rdma_resource_type }}   # "rdma/ib" or "rdma/roce_gdr"
{{ sriov_network }}        # for RoCE
{{ ucx_tls }}
{{ ucx_gid_index }}
{{ image }}
{{ gpu_count }}
{{ request_gpu }}
{{ namespace }}
{{ server_ip }}
{{ configmap_files }}      # filename -> base64
{{ extra_env_vars }}
```

## Example

See `manifests/06_deepep_low_latency/`:
- `manifest.yaml.j2:45` references `/opt/deepep-test/master-entrypoint.sh`
- `master-entrypoint.sh` gets embedded automatically
- `src/workloads/deepep_low_latency.rs:44` calls `.with_embedded_files("06_deepep_low_latency")`

## Build verification

```bash
cargo build
```

Output shows what got embedded:
```
Scanning manifests directory: "manifests"
  06_deepep_low_latency: 2 files (4523 bytes)
  01_nixl_transfer: 2 files (8912 bytes)
```

Missing files trigger a runtime warning:
```
No embedded files found for workload 'my-test'. ConfigMap will be empty.
```

## Adding a workload

1. Create `manifests/07_my_test/`
2. Add `manifest.yaml.j2`
3. Add scripts/configs
4. Implement `src/workloads/my_test.rs`
5. Call `.with_embedded_files("07_my_test")`
6. Reference files in manifest via volumeMounts

Files are embedded at build time and available in pods.
