#!/bin/bash
set -e
echo "Starting DeepEP internode test (WORKER) on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L

echo "GPU_COUNT from environment: $GPU_COUNT"

# source and run diagnostics
source /opt/deepep-test/diagnostics.sh
print_rdma_diagnostics

# detect and configure RDMA devices for dual SR-IOV
detect_dual_rdma_devices

if [ -n "$NET1_RDMA_DEVICE" ] && [ -n "$NET2_RDMA_DEVICE" ]; then
  # both libraries use both devices (shared HCA)
  export NCCL_IB_HCA="$NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"
  export NVSHMEM_HCA_LIST="$NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"
  export NVSHMEM_ENABLE_NIC_PE_MAPPING="1"
  export UCX_NET_DEVICES="$NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"

  # NVSHMEM bootstrap coordination on net1
  export NVSHMEM_BOOTSTRAP_UID_SOCK_IFNAME="net1"

  echo "=== Dual HCA Configuration (shared) ==="
  echo "  NCCL uses:    $NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"
  echo "  NVSHMEM uses: $NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"
  echo "  UCX uses:     $NET1_RDMA_DEVICE:1,$NET2_RDMA_DEVICE:1"
  echo "  NVSHMEM NIC mapping: enabled"
  echo "  NVSHMEM bootstrap: net1"
  echo "======================================="
else
  echo "ERROR: Failed to detect dual RDMA devices, cannot proceed"
  exit 1
fi

echo "Cloning DeepEP repository..."
cd /tmp
git clone https://github.com/deepseek-ai/DeepEP || echo "Repository already exists"
cd DeepEP
git checkout v1.2.1

# copy custom test.py script that supports 1, 2, 4, or 8 local ranks
echo "Installing custom test.py script..."
cp /opt/deepep-test/test.py tests/test_internode.py

echo "Waiting for master to be ready..."
until getent hosts "deepep-internode-master-${TEST_ID}"; do
  echo "Waiting for master service..."
  sleep 2
done
sleep 5

echo "Running DeepEP internode test with $GPU_COUNT processes per node, 2 nodes total..."
export MASTER_ADDR=deepep-internode-master-${TEST_ID}
export MASTER_PORT=29500
export WORLD_SIZE=2
export RANK=1
export PYTHONUNBUFFERED=1

echo "Starting Python test script..."
echo "Command: python tests/test_internode.py --num-processes $GPU_COUNT --num-tokens 512 --hidden 2048 --num-topk 4 --num-experts 32"

python -u tests/test_internode.py --num-processes "$GPU_COUNT" --num-tokens 512 --hidden 2048 --num-topk 4 --num-experts 32 2>&1 | tee /tmp/test_output.log

TEST_EXIT_CODE=${PIPESTATUS[0]}
echo "Python test exited with code: $TEST_EXIT_CODE"

if [ $TEST_EXIT_CODE -eq 0 ]; then
  echo "DeepEP internode test completed successfully"
else
  echo "DeepEP internode test FAILED with exit code $TEST_EXIT_CODE"
  echo "Last 50 lines of output:"
  tail -50 /tmp/test_output.log
  exit $TEST_EXIT_CODE
fi
