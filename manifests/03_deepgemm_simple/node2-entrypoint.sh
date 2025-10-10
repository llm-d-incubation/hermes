#!/bin/bash
set -e
echo "Starting DeepGEMM simple test on node ${NODE_NAME}"

echo "GPU information:"
nvidia-smi -L || echo "nvidia-smi not available"

echo "Running DeepGEMM simple test..."
python3 /opt/deepgemm-test/deepgemm-simple-test.py --gpu 0

echo "DeepGEMM simple test completed successfully"
