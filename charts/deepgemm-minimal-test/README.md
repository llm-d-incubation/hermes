# DeepGemm Minimal Test Helm Chart

This Helm chart deploys a minimal DeepGemm GPU compute test across two Kubernetes nodes. The test validates that the DeepGemm library is available and can perform basic operations.

## Overview

The DeepGemm minimal test runs on two nodes independently and validates:
- GPU and CUDA availability
- DeepGemm library import
- DeepGemm testing module availability
- Basic FP8 tensor operations
- Basic CUDA GEMM operations

**Note**: This is a compute-only test - it does not require RDMA resources or network communication between nodes.

## Prerequisites

- Kubernetes cluster with GPU nodes
- NVIDIA GPU operator or device plugin installed
- Container image with DeepGemm, PyTorch, and CUDA support

## Installation

### Using hermes (recommended)

The easiest way to deploy this test is via the hermes cluster analyzer:

```bash
# scan cluster and select optimal node pair
hermes select-nodes --format helm-values > values.yaml

# install the chart
helm install deepgemm-test ./charts/deepgemm-minimal-test -f values.yaml
```

### Manual Installation

```bash
# install with default values
helm install deepgemm-test ./charts/deepgemm-minimal-test

# install with custom values
helm install deepgemm-test ./charts/deepgemm-minimal-test \
  --set testId=my-test \
  --set namespace=gpu-tests \
  --set image=my-registry/cuda-dev:latest
```

## Configuration

Key configuration values:

| Parameter | Description | Default |
|-----------|-------------|---------|
| `testId` | Unique test identifier | `manual-test` |
| `namespace` | Kubernetes namespace | `default` |
| `activeDeadlineSeconds` | Job timeout | `180` |
| `image` | Container image with DeepGemm | See values.yaml |
| `resources.gpu` | GPU resource name | `nvidia.com/gpu` |
| `resources.requests.memory` | Memory request | `4Gi` |
| `resources.requests.cpu` | CPU request | `2` |
| `resources.limits.memory` | Memory limit | `8Gi` |
| `resources.limits.cpu` | CPU limit | `4` |
| `topology.nodes` | Array of node configs | See values.yaml |

## Usage

### Check test status

```bash
# view logs from both nodes
kubectl logs -n default -l app=deepgemm-minimal-test,role=node1
kubectl logs -n default -l app=deepgemm-minimal-test,role=node2

# check job status
kubectl get jobs -n default -l app=deepgemm-minimal-test
```

### Run Helm tests

```bash
helm test deepgemm-test -n default
```

This will wait for both jobs to complete and verify success messages in logs.

### Expected output

Successful test output includes:
```
DeepGEMM Minimal Availability Test
==================================================

GPU Info:
  ✅ CUDA available
  🔢 Device count: 8
  📱 Current device: 0
  🏷️  Device name: NVIDIA H100 80GB HBM3

Library Import:
  ✅ DeepGEMM imported successfully
  📍 Library path: ['/opt/venv/lib/python3.10/site-packages/deep_gemm']

Testing Module:
  ✅ Testing module imported successfully

Basic Tensor Ops:
  ✅ FP8 tensor creation successful: torch.float8_e4m3fn
  ✅ Basic CUDA GEMM successful: torch.Size([16, 16])

==================================================
SUMMARY
==================================================
GPU Info          - ✅ PASSED
Library Import    - ✅ PASSED
Testing Module    - ✅ PASSED
Basic Tensor Ops  - ✅ PASSED

Total: 4, Passed: 4, Failed: 0

✅ All availability tests passed!
DeepGEMM is ready for use!
```

## Troubleshooting

### Jobs fail to schedule

Check if nodes have GPU capacity:
```bash
kubectl describe node <node-name> | grep -A 5 "Allocated resources"
```

### DeepGemm import fails

Verify the container image includes DeepGemm:
```bash
kubectl exec -n default -l role=node1 -- python3 -c "import deep_gemm; print(deep_gemm.__version__)"
```

### GPU not detected

Check NVIDIA device plugin:
```bash
kubectl get pods -n kube-system -l name=nvidia-device-plugin-ds
kubectl logs -n kube-system -l name=nvidia-device-plugin-ds
```

## Cleanup

```bash
helm uninstall deepgemm-test -n default
```

This removes all jobs, ConfigMaps, and pods created by the chart.

## Architecture

The chart creates:
1. **ConfigMap**: Contains test scripts (node1-entrypoint.sh, node2-entrypoint.sh, deepgemm-minimal-test.py)
2. **Job (node1)**: Runs test on first selected node
3. **Job (node2)**: Runs test on second selected node
4. **Test Pod**: Helm test hook to validate completion

Both jobs run independently and do not communicate - this is purely a library availability test.

## Development

### Linting

```bash
cd charts/deepgemm-minimal-test
helm lint .
```

### Template validation

```bash
helm template test-release . --dry-run
```

### Local testing

```bash
# render with custom values
helm template test-release . \
  --set testId=local-test \
  --set topology.nodes[0].name=gpu-node-1 \
  --set topology.nodes[1].name=gpu-node-2
```

## License

Apache-2.0 (see test script for NVIDIA copyright notice)
