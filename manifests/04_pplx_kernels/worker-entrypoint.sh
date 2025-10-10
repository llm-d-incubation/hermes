#!/bin/bash
set -e
echo "Starting pplx-kernels all-to-all benchmark (WORKER) on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L

GPU_COUNT=$(nvidia-smi -L | wc -l)
echo "Detected $GPU_COUNT GPUs"

echo "Cloning pplx-kernels repository..."
cd /tmp
git clone https://github.com/perplexityai/pplx-kernels.git || echo "Repository already exists"
cd pplx-kernels

echo "Installing pytest to /tmp..."
pip install --target=/tmp pytest --quiet
export PYTHONPATH=/tmp:$PYTHONPATH

echo "Waiting for master to be ready..."
until getent hosts "pplx-kernels-master-${TEST_ID}"; do
  echo "Waiting for master service..."
  sleep 2
done
sleep 5

TOTAL_GPUS=$((GPU_COUNT * 2))
DP_SIZE=1
echo "Running all-to-all benchmark with world-size=$TOTAL_GPUS dp-size=$DP_SIZE (rank $GPU_COUNT-$((TOTAL_GPUS-1)))"

export MASTER_ADDR=pplx-kernels-master-${TEST_ID}
export MASTER_PORT=29500
export WORLD_SIZE=$TOTAL_GPUS
export WORLD_LOCAL_SIZE=$GPU_COUNT
export NODE_RANK=1
export RANK=$GPU_COUNT
export LOCAL_RANK=0

python -m tests.bench_all_to_all --dp-size $DP_SIZE

echo "pplx-kernels benchmark completed successfully"
