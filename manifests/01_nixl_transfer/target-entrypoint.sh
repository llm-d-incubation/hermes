#!/bin/bash
set -e
echo "Starting NIXL target on node ${NODE_NAME}"
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

echo "Checking IPC/SHM capabilities:"
ls -la /dev/shm/ || echo "No /dev/shm"
df -h | grep -E "(shm|tmp)" || echo "No shm mounts"
echo ""
echo "Container capabilities:"
capsh --print || echo "capsh not available"
echo ""

echo "Starting NIXL target script..."
/opt/vllm/bin/python3 /opt/nixl-test/nixl-transfer-test.py target 0.0.0.0 18515
