#!/bin/bash
set -e
echo "Starting DeepEP internode test (WORKER) on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L

echo "GPU_COUNT from environment: $GPU_COUNT"

echo "Cloning DeepEP repository..."
cd /tmp
git clone https://github.com/deepseek-ai/DeepEP || echo "Repository already exists"
cd DeepEP

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
export PYTHONUNBUFFERED=1

echo "Starting Python test script..."
echo "Command: python tests/test_internode.py --num-processes $GPU_COUNT --num-tokens 512 --hidden 1024 --num-topk 4 --num-experts 32"

python -u tests/test_internode.py --num-processes "$GPU_COUNT" --num-tokens 512 --hidden 1024 --num-topk 4 --num-experts 32 2>&1 | tee /tmp/test_output.log

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
