#!/bin/bash
set -e
echo "Starting DeepGEMM minimal test on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L || echo "nvidia-smi not available"

echo "Running DeepGEMM minimal test..."
python3 /opt/deepgemm-test/deepgemm-minimal-test.py --gpu 0

echo "DeepGEMM minimal test completed successfully"
