#!/bin/bash
set -e
echo "Starting NIXL initiator on node ${NODE_NAME}"
echo "Python environment check:"
which python3
python3 --version
echo "Checking installed packages:"
uv pip list | grep -i nixl || echo "Warning: NIXL not found in pip list"
echo "NIXL module contents:"
python3 -c "import nixl; print([x for x in dir(nixl) if not x.startswith('_')])" || echo "Error: Cannot inspect nixl"

echo "Network configuration:"
ip addr show

echo "RDMA devices:"
ls -la /dev/infiniband/ || echo "No infiniband devices found"

echo "Waiting 20 seconds for target to be ready..."
sleep 20

echo "Starting NIXL initiator script..."
/opt/vllm/bin/python3 /opt/nixl-test/nixl-transfer-test.py initiator "${TARGET_HOST}" "${TARGET_PORT}"

echo "NIXL transfer test completed"
