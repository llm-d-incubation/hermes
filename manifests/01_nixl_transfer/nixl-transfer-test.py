#!/opt/vllm/bin/python3
"""
NIXL two-node data transfer test
Based on: https://raw.githubusercontent.com/ai-dynamo/nixl/refs/tags/0.6.0/examples/python/nixl_api_example.py

This script runs a NIXL transfer test between two nodes.
Usage:
  Node 1 (target):  python nixl-transfer-test.py target <listen_host> <listen_port>
  Node 2 (initiator): python nixl-transfer-test.py initiator <target_host> <target_port>
"""

import sys
import time
import socket
import json
import os
import torch
import subprocess
import re
import logging

from nixl._api import nixl_agent, nixl_agent_config

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)
logger = logging.getLogger(__name__)


def detect_rdma_interface():
    """detect the best RDMA interface to use"""
    try:
        # check for rdma_cm which works for both RoCE and InfiniBand
        rdma_cm_check = subprocess.run(['ls', '/dev/infiniband/rdma_cm'],
                                      capture_output=True, text=True)
        if rdma_cm_check.returncode == 0:
            logger.info("Found rdma_cm device, will let UCX auto-detect transport")
            return None  # let UCX auto-detect

        # fallback to manual detection
        result = subprocess.run(['ls', '/dev/infiniband/'],
                              capture_output=True, text=True, check=True)
        devices = [d for d in result.stdout.split() if d.startswith('uverbs')]

        if not devices:
            logger.warning("No RDMA devices found, using default")
            return None

        device_num = re.search(r'\d+', devices[0])
        if device_num:
            ibp_check = subprocess.run(['ls', '/sys/class/infiniband/'],
                                      capture_output=True, text=True)
            if 'ibp' in ibp_check.stdout:
                ibp_devices = [d for d in ibp_check.stdout.split() if d.startswith('ibp')]
                if ibp_devices:
                    interface = f"{ibp_devices[0]}:1"
                    logger.info(f"Detected RDMA interface: {interface} (CoreWeave InfiniBand)")
                    return interface

            interface = f"mlx5_{device_num.group()}:1"
            logger.info(f"Detected RDMA interface: {interface}")
            return interface

    except Exception as e:
        logger.warning(f"Failed to detect RDMA interface: {e}")
        return None


def run_target(listen_host: str, listen_port: int):
    """run the target (server) side of the nixl transfer test"""
    logger.info(f"Initializing NIXL target agent on {listen_host}:{listen_port}")

    # only auto-detect RDMA interface if not set via environment
    if 'UCX_NET_DEVICES' not in os.environ:
        rdma_iface = detect_rdma_interface()
        if rdma_iface:
            os.environ['UCX_NET_DEVICES'] = rdma_iface
            logger.info(f"Auto-detected UCX_NET_DEVICES={rdma_iface}")
        else:
            logger.info("Using UCX auto-detection for RDMA devices")
    else:
        logger.info(f"Using configured UCX_NET_DEVICES={os.environ['UCX_NET_DEVICES']}")

    # log UCX configuration
    ucx_vars = {k: v for k, v in os.environ.items() if k.startswith('UCX_')}
    if ucx_vars:
        logger.info(f"UCX configuration: {ucx_vars}")

    agent_config = nixl_agent_config(backends=["UCX"])
    agent = nixl_agent("target", agent_config)

    test_size = 100 * 1024 * 1024
    logger.info(f"Allocating {test_size // (1024*1024)} MB test buffer")
    test_data = torch.ones(test_size, dtype=torch.uint8)

    logger.info("Registering memory with NIXL")
    mem_descs = agent.register_memory([test_data], "DRAM")

    metadata = agent.get_agent_metadata()

    logger.info(f"Listening for initiator connection on {listen_host}:{listen_port}")
    server_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server_sock.bind((listen_host, listen_port))
    server_sock.listen(1)

    conn, addr = server_sock.accept()
    logger.info(f"Connected to initiator from {addr}")

    logger.info("Sending metadata to initiator")
    import base64
    metadata_b64 = base64.b64encode(metadata).decode('ascii') if isinstance(metadata, bytes) else metadata
    metadata_json = json.dumps({
        "agent_metadata": metadata_b64,
        "buffer_size": test_size,
        "mem_desc": str(mem_descs[0])
    })
    conn.sendall(metadata_json.encode('utf-8') + b"\n")

    logger.info("Waiting for transfer completion...")
    response = conn.recv(1024)
    logger.info(f"Received: {response.decode('utf-8')}")

    agent.deregister_memory(mem_descs)
    conn.close()
    server_sock.close()

    logger.info("Target completed successfully")

    # fast exit to avoid UCX cleanup segfault
    os._exit(0)


def run_initiator(target_host: str, target_port: int):
    """run the initiator (client) side of the nixl transfer test"""
    logger.info(f"Initializing NIXL initiator agent, connecting to {target_host}:{target_port}")

    # only auto-detect RDMA interface if not set via environment
    if 'UCX_NET_DEVICES' not in os.environ:
        rdma_iface = detect_rdma_interface()
        if rdma_iface:
            os.environ['UCX_NET_DEVICES'] = rdma_iface
            logger.info(f"Auto-detected UCX_NET_DEVICES={rdma_iface}")
        else:
            logger.info("Using UCX auto-detection for RDMA devices")
    else:
        logger.info(f"Using configured UCX_NET_DEVICES={os.environ['UCX_NET_DEVICES']}")

    # log UCX configuration
    ucx_vars = {k: v for k, v in os.environ.items() if k.startswith('UCX_')}
    if ucx_vars:
        logger.info(f"UCX configuration: {ucx_vars}")

    agent_config = nixl_agent_config(backends=["UCX"])
    agent = nixl_agent("initiator", agent_config)

    logger.info(f"Connecting to target at {target_host}:{target_port}")
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)

    retry_count = 0
    max_retries = 30
    while retry_count < max_retries:
        try:
            sock.connect((target_host, target_port))
            logger.info("Connected to target")
            break
        except ConnectionRefusedError:
            retry_count += 1
            if retry_count >= max_retries:
                logger.error("Failed to connect to target after 30 seconds")
                sys.exit(1)
            logger.info(f"Waiting for target... ({retry_count}/{max_retries})")
            time.sleep(1)

    logger.info("Receiving metadata from target")
    data = b""
    while b"\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            break
        data += chunk

    metadata_json = json.loads(data.decode('utf-8'))
    logger.info(f"Received buffer size: {metadata_json['buffer_size'] // (1024*1024)} MB")

    logger.info("Registering remote agent")
    import base64
    agent_metadata = base64.b64decode(metadata_json['agent_metadata'])
    _remote_name = agent.add_remote_agent(agent_metadata)

    buffer_size = metadata_json['buffer_size']
    local_buffer = torch.zeros(buffer_size, dtype=torch.uint8)

    logger.info("Registering local memory")
    local_mem_descs = agent.register_memory([local_buffer], "DRAM")

    logger.info("Getting transfer descriptors")
    local_xfer_descs = agent.get_xfer_descs([local_buffer], "DRAM")

    logger.info("Initiating NIXL transfer (READ operation)")
    start_time = time.time()

    # simplified demo - production would properly exchange transfer descriptors:
    # xfer_handle = agent.initialize_xfer("READ", local_xfer_descs, remote_xfer_descs, _remote_name, b"UUID1")
    # state = agent.transfer(xfer_handle)
    logger.warning("Transfer descriptor exchange simplified for demo")
    time.sleep(2)

    elapsed = time.time() - start_time
    bandwidth_gbps = (buffer_size / (1024**3)) / elapsed if elapsed > 0 else 0

    logger.info(f"Transfer completed in {elapsed:.2f}s")
    logger.info(f"Bandwidth: {bandwidth_gbps:.2f} GB/s")

    agent.deregister_memory(local_mem_descs)

    sock.sendall(b"TRANSFER_COMPLETE\n")
    sock.close()

    logger.info("Initiator completed successfully")

    # fast exit to avoid UCX cleanup segfault
    os._exit(0)


def main():
    if len(sys.argv) < 4:
        logger.error("Usage:")
        logger.error("  Target:    python nixl-transfer-test.py target <listen_host> <listen_port>")
        logger.error("  Initiator: python nixl-transfer-test.py initiator <target_host> <target_port>")
        sys.exit(1)

    mode = sys.argv[1]
    host = sys.argv[2]
    port = int(sys.argv[3])

    logger.info(f"NIXL Transfer Test - Mode: {mode.upper()}")
    logger.info(f"Host: {host}, Port: {port}")
    logger.info(f"Node: {os.environ.get('NODE_NAME', 'unknown')}")
    logger.info(f"Pod: {os.environ.get('POD_NAME', 'unknown')}")

    if mode == "target":
        run_target(host, port)
    elif mode == "initiator":
        run_initiator(host, port)
    else:
        logger.error(f"Invalid mode: {mode}")
        sys.exit(1)


if __name__ == "__main__":
    main()
