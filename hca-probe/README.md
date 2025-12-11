# hca-probe

Rust replacement for KServe's [detect roce shell script](https://github.com/red-hat-data-services/kserve/blob/c832f31a0a7c78bb2dbdfa3b6e5783a306abdceb/config/llmisvc/config-llm-template.yaml#L19-L138). Uses ibverbs API instead of sysfs parsing, properly distinguishes InfiniBand from RoCE, and filters VFs.

## Entrypoint Usage

```bash
#!/bin/bash
# IB-only, no VFs
eval "$(hca-probe detect -f env -l ib --no-vf 2>/dev/null | grep '^export')"
exec "$@"
```

## Help

```
Usage: hca-probe <command> [<args>]

Detect and configure RDMA HCAs (InfiniBand and RoCE) for NCCL, NVSHMEM, and UCX

Options:
  --help, help      display usage information

Commands:
  detect            Detect RDMA HCAs and output configuration
  iface-hca         Map network interfaces to InfiniBand HCAs
  vf-map            Map SR-IOV Virtual Functions to Physical Functions
  iommu-acs         Check IOMMU and PCI ACS configuration
  iface-ip          List network interfaces with IP addresses
```

### detect

```
Usage: hca-probe detect [-f <format>] [-i <socket-ifname>] [-g <gid-index>] [-p <device-prefix>] [-l <link-layer>] [--no-vf] [--namespace-pid <namespace-pid>] [--namespace-id <namespace-id>]

Detect RDMA HCAs and output configuration

Options:
  -f, --format      output format: env, json, or quiet
  -i, --socket-ifname
                    filter HCAs by network interface name (comma-separated)
  -g, --gid-index   force a specific GID index (overrides auto-detection)
  -p, --device-prefix
                    device prefix to filter (e.g., "mlx5_", "mlx4_", "bnxt_")
  -l, --link-layer  filter by link layer: ib, roce, or all
  --no-vf           exclude SR-IOV Virtual Functions (VFs)
  --namespace-pid   enter network namespace of specific PID before detection
  --namespace-id    namespace identifier for output correlation
  --help, help      display usage information
```
