#!/bin/bash
# shared RDMA diagnostics script

# returns the RDMA device name that corresponds to SR-IOV interface
# uses SRIOV_INTERFACE env var (defaults to net1)
get_sriov_rdma_device() {
  local iface="${SRIOV_INTERFACE:-net1}"

  if [ -d "/sys/class/net/$iface" ]; then
    local iface_pci
    iface_pci=$(readlink -f "/sys/class/net/$iface/device" 2>/dev/null | xargs basename)
    for dev in /sys/class/infiniband/*; do
      if [ -d "$dev" ]; then
        dev_name=$(basename "$dev")
        dev_pci=$(readlink -f "$dev/device" 2>/dev/null | xargs basename)
        if [ "$dev_pci" = "$iface_pci" ]; then
          echo "$dev_name"
          return 0
        fi
      fi
    done
  fi
  return 1
}

print_rdma_diagnostics() {
  echo ""
  echo "=== RDMA Device Diagnostics ==="
  echo "Network interfaces:"
  ip addr show | grep -E "^[0-9]+:|inet "

  echo ""
  echo "InfiniBand/RoCE devices:"
  ls -la /sys/class/infiniband/ || echo "No IB devices found"

  echo ""
  echo "RDMA device details:"
  for dev in /sys/class/infiniband/*; do
    if [ -d "$dev" ]; then
      dev_name=$(basename "$dev")
      echo "Device: $dev_name"

      # show GID table for port 1
      if [ -d "$dev/ports/1/gids" ]; then
        echo "  GID table (port 1):"
        for i in {0..5}; do
          if [ -f "$dev/ports/1/gids/$i" ]; then
            gid=$(cat "$dev/ports/1/gids/$i" 2>/dev/null || echo "error")
            echo "    [$i] $gid"
          fi
        done
      fi

      # try to find associated network interface
      if [ -d "$dev/device/net" ]; then
        netdev=$(ls "$dev/device/net" 2>/dev/null | head -1)
        if [ -n "$netdev" ]; then
          echo "  Network interface: $netdev"
          ip addr show "$netdev" | grep -E "inet |link/ether"
        fi
      fi
      echo ""
    fi
  done

  echo "NCCL environment variables:"
  env | grep NCCL || echo "No NCCL env vars set"

  echo ""
  echo "Network interface to RDMA device mapping:"
  if command -v ibdev2netdev &> /dev/null; then
    ibdev2netdev
  else
    echo "ibdev2netdev not available, using sysfs mapping:"
    for netif in /sys/class/net/*; do
      ifname=$(basename "$netif")
      if [ -L "$netif/device/infiniband" ]; then
        ibdev=$(ls "$netif/device/infiniband" 2>/dev/null | head -1)
        pci=$(readlink -f "$netif/device" | xargs basename)
        echo "  $ifname -> $ibdev (PCI: $pci)"
      fi
    done
  fi

  echo ""
  echo "GPU topology:"
  nvidia-smi topo -m 2>/dev/null || echo "GPU topology not available"

  local iface="${SRIOV_INTERFACE:-net1}"
  echo ""
  echo "Checking SR-IOV interface ($iface):"
  if [ -d "/sys/class/net/$iface" ]; then
    echo "  $iface exists"
    local iface_pci
    iface_pci=$(readlink -f "/sys/class/net/$iface/device" 2>/dev/null | xargs basename)
    echo "  PCI address: $iface_pci"

    # find matching RDMA device
    for dev in /sys/class/infiniband/*; do
      if [ -d "$dev" ]; then
        dev_name=$(basename "$dev")
        dev_pci=$(readlink -f "$dev/device" 2>/dev/null | xargs basename)
        if [ "$dev_pci" = "$iface_pci" ]; then
          echo "  Matching RDMA device: $dev_name"
        fi
      fi
    done
  else
    echo "  $iface not found!"
  fi

  echo "=== End Diagnostics ==="
  echo ""
}
