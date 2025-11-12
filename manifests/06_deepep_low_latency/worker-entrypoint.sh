#!/bin/bash
set -e
echo "Starting DeepEP low latency test (WORKER) on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L

GPU_COUNT=$(nvidia-smi -L | wc -l)
echo "Detected $GPU_COUNT GPUs"

# source and run diagnostics
source /opt/deepep-test/diagnostics.sh
print_rdma_diagnostics

# detect and configure RDMA device for nvshmem and NCCL
RDMA_DEVICE=$(get_sriov_rdma_device)
if [ -n "$RDMA_DEVICE" ]; then
  export NVSHMEM_HCA_LIST="$RDMA_DEVICE"
  export NCCL_IB_HCA="$RDMA_DEVICE"
  echo "Configured NVSHMEM_HCA_LIST and NCCL_IB_HCA to use RDMA device: $RDMA_DEVICE (interface: ${SRIOV_INTERFACE:-net1})"
else
  echo "WARNING: Could not detect RDMA device for SR-IOV interface ${SRIOV_INTERFACE:-net1}, RDMA operations may fail"
fi

echo "Cloning DeepEP repository..."
cd /tmp
git clone https://github.com/deepseek-ai/DeepEP || echo "Repository already exists"
cd DeepEP
git checkout v1.2.1

echo "Waiting for master to be ready..."
until getent hosts "deepep-lowlatency-master-${TEST_ID}"; do
  echo "Waiting for master service..."
  sleep 2
done
sleep 5

TOTAL_GPUS=$((GPU_COUNT * 2))
echo "Running DeepEP low latency test with $TOTAL_GPUS total GPUs (rank $GPU_COUNT-$((TOTAL_GPUS-1)))"

export MASTER_ADDR=deepep-lowlatency-master-${TEST_ID}
export MASTER_PORT=29500
export WORLD_SIZE=$TOTAL_GPUS
export RANK=$GPU_COUNT
export PYTHONUNBUFFERED=1

echo "Starting Python test script..."
echo "Command: python tests/test_low_latency.py --num-processes $GPU_COUNT --num-tokens 128 --hidden 2048 --num-topk 4 --num-experts 32"

python -u tests/test_low_latency.py --num-processes "$GPU_COUNT" --num-tokens 128 --hidden 2048 --num-topk 4 --num-experts 32 2>&1 | tee /tmp/test_output.log

TEST_EXIT_CODE=${PIPESTATUS[0]}
echo "Python test exited with code: $TEST_EXIT_CODE"

if [ $TEST_EXIT_CODE -eq 0 ]; then
  echo "DeepEP low latency test completed successfully"
else
  echo "DeepEP low latency test FAILED with exit code $TEST_EXIT_CODE"
  echo "Last 50 lines of output:"
  tail -50 /tmp/test_output.log
  exit $TEST_EXIT_CODE
fi
