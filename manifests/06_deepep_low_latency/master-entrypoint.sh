#!/bin/bash
set -e
echo "Starting DeepEP low latency test (MASTER) on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L

GPU_COUNT=$(nvidia-smi -L | wc -l)
echo "Detected $GPU_COUNT GPUs"

echo "Cloning DeepEP repository..."
cd /tmp
git clone https://github.com/deepseek-ai/DeepEP || echo "Repository already exists"
cd DeepEP

TOTAL_GPUS=$((GPU_COUNT * 2))
echo "Running DeepEP low latency test with $TOTAL_GPUS total GPUs (rank 0-$((GPU_COUNT-1)))"

export MASTER_ADDR=deepep-lowlatency-master-${TEST_ID}
export MASTER_PORT=29500
export WORLD_SIZE=$TOTAL_GPUS
export RANK=0
export PYTHONUNBUFFERED=1

echo "Starting Python test script..."
echo "Command: python tests/test_low_latency.py --num-processes $GPU_COUNT --num-tokens 128 --hidden 1024 --num-topk 4 --num-experts 32"

python -u tests/test_low_latency.py --num-processes "$GPU_COUNT" --num-tokens 128 --hidden 1024 --num-topk 4 --num-experts 32 2>&1 | tee /tmp/test_output.log

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
