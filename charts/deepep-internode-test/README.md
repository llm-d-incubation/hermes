# DeepEP Internode Test Helm Chart

Helm chart for deploying DeepSeek's DeepEP multi-node inter-node communication test on Kubernetes.

## Overview

This chart deploys a distributed test of DeepEP (Expert Parallelism library) across multiple GPU nodes with RDMA networking. It creates:

- **1 Master Job** (rank 0): Coordinates distributed test and runs first rank
- **N-1 Worker Jobs**: Additional ranks running on separate nodes
- **1 Headless Service**: DNS-based rendezvous for PyTorch distributed coordination
- **1 ConfigMap**: Test scripts, diagnostics, and custom DeepEP test code

## Features

### Multi-Node Coordination
- Supports variable number of nodes (not limited to 2)
- Automatic worker job creation via Helm range loops
- Master/worker coordination via Kubernetes headless service
- Per-rank environment variable configuration

### RDMA Support
- Dual HCA configuration (2 RDMA devices per pod)
- InfiniBand and RoCE support
- SR-IOV network attachment for RoCE deployments
- UCX transport configuration

### GPU & Accelerator
- Configurable GPUs per node (1, 2, 4, or 8)
- NCCL and NVSHMEM configuration
- Dual HCA support for both libraries
- Shared memory (shm) volume for inter-process communication

## Prerequisites

- Kubernetes cluster with RDMA-capable nodes
- NVIDIA GPUs with GPUDirect RDMA support
- RDMA resources advertised (e.g., `rdma/ib`, `rdma/roce_gdr`)
- For RoCE: SR-IOV network operator configured

## Installation

### Basic Installation (InfiniBand)

```bash
helm install deepep-test charts/deepep-internode-test/ \
  --set testId=my-test \
  --set namespace=default \
  --set-json 'topology.nodes=[
    {"name":"node1","gpus":8,"rank":0,"topologyBlock":"zone-a"},
    {"name":"node2","gpus":8,"rank":1,"topologyBlock":"zone-a"}
  ]'
```

### RoCE Deployment with SR-IOV

```bash
helm install deepep-roce-test charts/deepep-internode-test/ \
  --set testId=roce-test \
  --set namespace=openshift-compute \
  --set sriov.enabled=true \
  --set-json 'sriov.networks=[
    {"name":"rdma-network-1"},
    {"name":"rdma-network-2"}
  ]' \
  --set ucx.gidIndex=3 \
  --set ucx.transports=rc,ud,tcp \
  --set resources.rdma=rdma/roce_gdr
```

### 4-Node Deployment

```bash
helm install deepep-4node charts/deepep-internode-test/ \
  --set testId=4node-test \
  --set deepep.worldSize=4 \
  --set-json 'topology.nodes=[
    {"name":"node-a","gpus":8,"rank":0,"topologyBlock":"zone-1"},
    {"name":"node-b","gpus":8,"rank":1,"topologyBlock":"zone-1"},
    {"name":"node-c","gpus":8,"rank":2,"topologyBlock":"zone-2"},
    {"name":"node-d","gpus":8,"rank":3,"topologyBlock":"zone-2"}
  ]'
```

## Configuration

### Key Values

| Parameter | Description | Default |
|-----------|-------------|---------|
| `testId` | Unique test identifier | `manual-test` |
| `namespace` | Kubernetes namespace | `default` |
| `activeDeadlineSeconds` | Job timeout | `600` (10 min) |
| `image` | Container image with DeepEP | `ghcr.io/llm-d/llm-d-cuda-dev:...` |
| `resources.gpuCount` | GPUs per pod (1,2,4,8) | `8` |
| `resources.rdmaQuantity` | RDMA devices per pod | `2` |
| `deepep.worldSize` | Number of nodes | `2` |
| `deepep.numTokens` | Test tokens | `512` |
| `deepep.numExperts` | Number of experts | `32` |
| `ucx.transports` | UCX transport list | `rc,ud,dc,tcp,...` |
| `ucx.gidIndex` | RDMA GID index | `0` (IB), `3` (RoCE) |
| `sriov.enabled` | Enable SR-IOV networks | `false` |

### Topology Configuration

The `topology.nodes` array defines the test cluster layout:

```yaml
topology:
  nodes:
    - name: node1          # Kubernetes node name
      gpus: 8              # GPUs on this node
      rank: 0              # PyTorch rank (0 = master)
      topologyBlock: zone-a  # Placement hint
    - name: node2
      gpus: 8
      rank: 1              # Worker ranks start at 1
      topologyBlock: zone-a
  summary:
    totalNodes: 2
    worldSize: 2           # Must match deepep.worldSize
```

**Important**: Rank 0 is always the master. Worker jobs are created for ranks 1 through N-1.

## Monitoring

### View Job Status

```bash
kubectl get jobs -n <namespace> -l test-id=<testId> --watch
```

### View Logs

```bash
# Master logs
kubectl logs -n <namespace> -l app=deepep-internode-test,role=master,test-id=<testId> -f

# All worker logs
kubectl logs -n <namespace> -l app=deepep-internode-test,role=worker,test-id=<testId> --all-containers -f

# Specific rank logs
kubectl logs -n <namespace> -l app=deepep-internode-test,rank=1,test-id=<testId> -f
```

### Check Resources

```bash
kubectl get pods,jobs,services -n <namespace> -l test-id=<testId>
```

## Test Execution Flow

1. **Master Job** (rank 0):
   - Starts immediately
   - Creates headless service for DNS resolution
   - Detects RDMA devices
   - Clones DeepEP repository
   - Runs DeepEP test as rank 0

2. **Worker Jobs** (ranks 1-N):
   - Wait for master service to be resolvable
   - Detect RDMA devices independently
   - Clone DeepEP repository
   - Connect to master via service DNS
   - Run DeepEP test with assigned rank

3. **Test Completion**:
   - Each job completes independently
   - Exit code 0 = success
   - Logs saved to `/tmp/test_output.log` in each pod

## Troubleshooting

### RDMA Device Detection Failed

```bash
kubectl exec -n <namespace> -it <pod-name> -- ls -la /sys/class/infiniband/
```

Check for:
- SR-IOV VF mode: `net1` and `net2` interfaces
- Multi-NIC mode: `net1-0` and `net1-1` interfaces

### Service Not Resolvable

```bash
kubectl exec -n <namespace> -it <worker-pod> -- getent hosts deepep-internode-master-<testId>
```

Ensure master pod has started and is running.

### GPU/RDMA Resource Scheduling

```bash
kubectl describe pod <pod-name> -n <namespace>
```

Check for resource constraints in events.

### SR-IOV Network Attachment Issues

Verify SR-IOV networks exist:

```bash
kubectl get sriovnetwork -n openshift-sriov-network-operator
```

Check network namespace matches deployment namespace.

## Cleanup

```bash
helm uninstall <release-name> -n <namespace>

# Or manually:
kubectl delete jobs,services,configmaps -n <namespace> -l test-id=<testId>
```

## Development

### Lint Chart

```bash
helm lint charts/deepep-internode-test/
```

### Template Test

```bash
helm template test charts/deepep-internode-test/ -f values-override.yaml
```

### Run Helm Tests

```bash
helm test <release-name> -n <namespace>
```

## Architecture Notes

### Master/Worker Pattern
- Not a StatefulSet (no strict ordering required)
- Jobs are independent after initial DNS resolution
- Worker ranks wait for master service but then proceed independently

### Rank Assignment
- Ranks are assigned via `topology.nodes[].rank`
- Master is always rank 0
- Workers use Helm range loop: `{{ range .Values.topology.nodes }}`
- Only creates jobs for `rank != 0`

### Rendezvous Backend
- Currently uses PyTorch's default TCP rendezvous via master service
- Master address: `deepep-internode-master-<testId>`
- Master port: `29500` (configurable via `deepep.rendezvous.masterPort`)

### Resource Considerations
- Each pod requests 2 RDMA devices for dual HCA setup
- Shared memory (shm) volume sized at 16Gi per pod
- Memory requests: 32Gi, limits: 64Gi per pod
- CPU requests: 16, limits: 32 per pod

## References

- [DeepEP GitHub](https://github.com/deepseek-ai/DeepEP)
- [Hermes Cluster Wizard](https://github.com/llm-d-incubation/llmd-cluster-wizard)
- [UCX Documentation](https://openucx.readthedocs.io/)
- [NCCL Documentation](https://docs.nvidia.com/deeplearning/nccl/)
- [NVSHMEM Documentation](https://docs.nvidia.com/nvshmem/)
