#!/bin/bash
# shared RDMA diagnostics script

# enumerate all physical RDMA NICs on the node
enumerate_physical_rdma_nics() {
  echo ""
  echo "========================================="
  echo "  PHYSICAL RDMA NIC ENUMERATION"
  echo "========================================="

  # count physical NICs (p0-p15 on bare metal, net1-* via multi-nic CNI)
  local physical_nics
  physical_nics=$(find /sys/class/net/ -maxdepth 1 -type l \( -name 'p[0-9]*' -o -name 'net1-[0-9]*' \) -print0 2>/dev/null | xargs -0 -n1 basename | sort -V)
  local nic_count
  nic_count=$(echo "$physical_nics" | wc -l)

  echo "Found $nic_count physical RDMA NICs:"
  echo ""

  # detailed mapping
  echo "NIC -> RDMA Device -> PCI Address -> Status"
  echo "-------------------------------------------"

  for nic in $physical_nics; do
    if [ -d "/sys/class/net/$nic" ]; then
      # get PCI address
      local pci
      pci=$(readlink -f "/sys/class/net/$nic/device" 2>/dev/null | xargs basename)

      # find matching RDMA device
      local rdma_dev=""
      for dev in /sys/class/infiniband/*; do
        if [ -d "$dev" ]; then
          local dev_name
          dev_name=$(basename "$dev")
          local dev_pci
          dev_pci=$(readlink -f "$dev/device" 2>/dev/null | xargs basename)
          if [ "$dev_pci" = "$pci" ]; then
            rdma_dev="$dev_name"
            break
          fi
        fi
      done

      # get link status
      local status="DOWN"
      if [ -f "/sys/class/net/$nic/operstate" ]; then
        local operstate
        operstate=$(cat "/sys/class/net/$nic/operstate" 2>/dev/null)
        if [ "$operstate" = "up" ]; then
          status="UP"
        fi
      fi

      printf "%-4s -> %-10s -> %s -> %s\n" "$nic" "${rdma_dev:-N/A}" "$pci" "$status"
    fi
  done

  echo ""
  echo "Total Physical RDMA NICs: $nic_count"

  # count GPUs
  local gpu_count
  gpu_count=$(nvidia-smi -L 2>/dev/null | wc -l)
  echo "Total GPUs: $gpu_count"

  if [ "$gpu_count" -gt 0 ] && [ "$nic_count" -gt 0 ]; then
    local nics_per_gpu=$((nic_count / gpu_count))
    echo "NICs per GPU: $nics_per_gpu"
  fi

  echo "========================================="
  echo ""
}

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

# detects RDMA devices for both net1 and net2 interfaces
# outputs: NET1_RDMA_DEVICE, NET2_RDMA_DEVICE, NET1_INTERFACE, NET2_INTERFACE variables
detect_dual_rdma_devices() {
  echo "Detecting RDMA devices for network interfaces..."

  # check for SR-IOV VF mode (net1, net2)
  if [ -d "/sys/class/net/net1" ] && [ -d "/sys/class/net/net2" ]; then
    echo "  Mode: SR-IOV VF (net1, net2)"
    NET1_INTERFACE="net1"
    NET2_INTERFACE="net2"
  # check for multi-nic-compute mode (net1-0, net1-1, ...)
  elif [ -d "/sys/class/net/net1-0" ] && [ -d "/sys/class/net/net1-1" ]; then
    echo "  Mode: multi-nic-compute (net1-0, net1-1, ...)"
    NET1_INTERFACE="net1-0"
    NET2_INTERFACE="net1-1"

    # for multi-nic-compute with IPVLAN, we can't use PCI mapping
    # gather all RDMA devices and filter for ones with valid GIDs
    local all_rdma_devs=()
    mapfile -t all_rdma_devs < <(find /sys/class/infiniband/ -maxdepth 1 -type l -name 'mlx*' -print0 2>/dev/null | xargs -0 -n1 basename | sort -V)
    local usable_devs=()
    local unusable_devs=()

    for dev in "${all_rdma_devs[@]}"; do
      # check if device has any non-zero GID on port 1
      local has_valid_gid=0
      if [ -d "/sys/class/infiniband/$dev/ports/1/gids" ]; then
        for gid_file in /sys/class/infiniband/$dev/ports/1/gids/*; do
          if [ -f "$gid_file" ]; then
            local gid
            gid=$(cat "$gid_file" 2>/dev/null)
            if [ -n "$gid" ] && [ "$gid" != "0000:0000:0000:0000:0000:0000:0000:0000" ]; then
              has_valid_gid=1
              break
            fi
          fi
        done
      fi

      if [ $has_valid_gid -eq 1 ]; then
        usable_devs+=("$dev")
      else
        unusable_devs+=("$dev")
      fi
    done

    if [ "${#usable_devs[@]}" -ge 2 ]; then
      # use first usable device for primary, second for NCCL
      NET1_RDMA_DEVICE="${usable_devs[0]}"
      NET2_RDMA_DEVICE="${usable_devs[1]}"

      # build exclusion list of unusable devices
      if [ "${#unusable_devs[@]}" -gt 0 ]; then
        EXCLUDED_RDMA_DEVICES=$(printf "%s:1," "${unusable_devs[@]}")
        EXCLUDED_RDMA_DEVICES="${EXCLUDED_RDMA_DEVICES%,}"  # remove trailing comma
      fi

      # build list of all usable devices for UCX
      ALL_RDMA_DEVICES=$(printf "%s:1," "${usable_devs[@]}")
      ALL_RDMA_DEVICES="${ALL_RDMA_DEVICES%,}"

      echo "  Detected ${#all_rdma_devs[@]} total RDMA devices (${#usable_devs[@]} usable, ${#unusable_devs[@]} excluded)"
      echo "  Usable devices: ${usable_devs[*]}"
      if [ "${#unusable_devs[@]}" -gt 0 ]; then
        echo "  Excluded (no valid GIDs): ${unusable_devs[*]}"
      fi
    fi

    # early return for multi-nic mode since we handled it above
    if [ -z "$NET1_RDMA_DEVICE" ] || [ -z "$NET2_RDMA_DEVICE" ]; then
      echo "WARNING: Could not detect RDMA devices in multi-nic mode"
      echo "  Available RDMA devices: $(find /sys/class/infiniband/ -maxdepth 1 -type l -name 'mlx*' -print0 2>/dev/null | xargs -0 -n1 basename | tr '\n' ' ')"
      return 1
    fi
    return 0
  else
    echo "ERROR: Could not detect network interface mode"
    echo "  Expected either:"
    echo "    - SR-IOV VF mode: net1 and net2"
    echo "    - multi-nic-compute mode: net1-0 and net1-1"
    return 1
  fi

  # detect rdma device for first interface (SR-IOV VF mode only)
  if [ -d "/sys/class/net/$NET1_INTERFACE" ]; then
    local net1_pci
    net1_pci=$(readlink -f "/sys/class/net/$NET1_INTERFACE/device" 2>/dev/null | xargs basename)
    for dev in /sys/class/infiniband/*; do
      if [ -d "$dev" ]; then
        dev_name=$(basename "$dev")
        dev_pci=$(readlink -f "$dev/device" 2>/dev/null | xargs basename)
        if [ "$dev_pci" = "$net1_pci" ]; then
          NET1_RDMA_DEVICE="$dev_name"
          echo "  $NET1_INTERFACE -> $NET1_RDMA_DEVICE (PCI: $net1_pci)"
          break
        fi
      fi
    done
  fi

  # detect rdma device for second interface (SR-IOV VF mode only)
  if [ -d "/sys/class/net/$NET2_INTERFACE" ]; then
    local net2_pci
    net2_pci=$(readlink -f "/sys/class/net/$NET2_INTERFACE/device" 2>/dev/null | xargs basename)
    for dev in /sys/class/infiniband/*; do
      if [ -d "$dev" ]; then
        dev_name=$(basename "$dev")
        dev_pci=$(readlink -f "$dev/device" 2>/dev/null | xargs basename)
        if [ "$dev_pci" = "$net2_pci" ]; then
          NET2_RDMA_DEVICE="$dev_name"
          echo "  $NET2_INTERFACE -> $NET2_RDMA_DEVICE (PCI: $net2_pci)"
          break
        fi
      fi
    done
  fi

  if [ -z "$NET1_RDMA_DEVICE" ] || [ -z "$NET2_RDMA_DEVICE" ]; then
    echo "WARNING: Could not detect both RDMA devices"
    echo "  $NET1_INTERFACE -> ${NET1_RDMA_DEVICE:-not found}"
    echo "  $NET2_INTERFACE -> ${NET2_RDMA_DEVICE:-not found}"
    return 1
  fi

  return 0
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
