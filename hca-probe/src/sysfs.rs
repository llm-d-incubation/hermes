//! sysfs utilities for network interface and RDMA device discovery

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const NET_CLASS: &str = "/sys/class/net";
pub const IB_CLASS: &str = "/sys/class/infiniband";
pub const PCI_DEVICES: &str = "/sys/bus/pci/devices";

/// check if a device is a Virtual Function by looking for physfn symlink
pub fn is_virtual_function(device_path: &Path) -> bool {
    device_path.join("physfn").is_symlink()
}

/// info about a network interface from sysfs
#[derive(Debug, Clone)]
pub struct NetInterface {
    pub name: String,
    pub has_device: bool,
    pub is_vf: bool,
    #[allow(dead_code)]
    pub pf_name: Option<String>,
    #[allow(dead_code)]
    pub pci_slot: Option<String>,
    pub mac_address: Option<String>,
    pub operstate: Option<String>,
}

/// info about an InfiniBand HCA from sysfs
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct HcaInfo {
    pub name: String,
    pub pci_slot: Option<String>,
    pub netdevs: Vec<String>,
}

/// enumerate all network interfaces from /sys/class/net
pub fn list_net_interfaces() -> Result<Vec<NetInterface>> {
    let net_path = Path::new(NET_CLASS);
    if !net_path.exists() {
        return Ok(Vec::new());
    }

    let mut interfaces = Vec::new();

    for entry in fs::read_dir(net_path).context("failed to read /sys/class/net")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // skip loopback
        if name == "lo" {
            continue;
        }

        let iface_path = entry.path();
        let device_path = iface_path.join("device");
        let has_device = device_path.is_symlink();

        let is_vf = has_device && is_virtual_function(&device_path);

        let pf_name = if is_vf {
            get_pf_name_for_vf(&iface_path)
        } else {
            None
        };

        let pci_slot = if has_device {
            get_pci_slot(&device_path)
        } else {
            None
        };

        let mac_address = read_sysfs_string(&iface_path.join("address"));
        let operstate = read_sysfs_string(&iface_path.join("operstate"));

        interfaces.push(NetInterface {
            name,
            has_device,
            is_vf,
            pf_name,
            pci_slot,
            mac_address,
            operstate,
        });
    }

    interfaces.sort_by(|a, b| natural_sort_key(&a.name).cmp(&natural_sort_key(&b.name)));
    Ok(interfaces)
}

/// enumerate InfiniBand HCAs from /sys/class/infiniband
#[allow(dead_code)]
pub fn list_hcas() -> Result<Vec<HcaInfo>> {
    let ib_path = Path::new(IB_CLASS);
    if !ib_path.exists() {
        return Ok(Vec::new());
    }

    let mut hcas = Vec::new();

    for entry in fs::read_dir(ib_path).context("failed to read /sys/class/infiniband")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let hca_path = entry.path();

        let device_path = hca_path.join("device");
        let pci_slot = if device_path.is_symlink() {
            get_pci_slot(&device_path)
        } else {
            None
        };

        let netdevs = get_hca_netdevs(&hca_path);

        hcas.push(HcaInfo {
            name,
            pci_slot,
            netdevs,
        });
    }

    hcas.sort_by(|a, b| natural_sort_key(&a.name).cmp(&natural_sort_key(&b.name)));
    Ok(hcas)
}

/// get the PF name for a VF interface
fn get_pf_name_for_vf(iface_path: &Path) -> Option<String> {
    let physfn_net = iface_path.join("device/physfn/net");
    if physfn_net.is_dir() {
        if let Ok(entries) = fs::read_dir(&physfn_net) {
            for entry in entries.flatten() {
                return Some(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    None
}

/// get VFs for a PF, including orphan VFs (no network interface)
pub fn get_vfs_for_pf(pf_iface_path: &Path) -> Vec<VfInfo> {
    let device_path = pf_iface_path.join("device");
    if !device_path.is_symlink() {
        return Vec::new();
    }

    let mut vfs = Vec::new();

    // check virtfn* symlinks
    if let Ok(entries) = fs::read_dir(&device_path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("virtfn") {
                continue;
            }

            let virtfn_path = entry.path();
            let net_path = virtfn_path.join("net");

            let iface_name = if net_path.is_dir() {
                fs::read_dir(&net_path)
                    .ok()
                    .and_then(|mut entries| entries.next())
                    .and_then(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
            } else {
                None
            };

            vfs.push(VfInfo {
                virtfn_index: name,
                iface_name,
            });
        }
    }

    vfs.sort_by(|a, b| natural_sort_key(&a.virtfn_index).cmp(&natural_sort_key(&b.virtfn_index)));
    vfs
}

#[derive(Debug, Clone)]
pub struct VfInfo {
    pub virtfn_index: String,
    pub iface_name: Option<String>,
}

/// get netdevs associated with an HCA by checking port gid_attrs
#[allow(dead_code)]
fn get_hca_netdevs(hca_path: &Path) -> Vec<String> {
    let mut netdevs = Vec::new();
    let ports_path = hca_path.join("ports");

    if let Ok(ports) = fs::read_dir(&ports_path) {
        for port in ports.flatten() {
            let ndevs_path = port.path().join("gid_attrs/ndevs");
            if let Ok(ndevs) = fs::read_dir(&ndevs_path) {
                for ndev in ndevs.flatten() {
                    if let Some(netdev) = read_sysfs_string(&ndev.path()) {
                        if !netdevs.contains(&netdev) {
                            netdevs.push(netdev);
                        }
                    }
                }
            }
        }
    }

    netdevs
}

/// extract PCI slot from device symlink (e.g., "0000:3b:00.0")
fn get_pci_slot(device_path: &Path) -> Option<String> {
    fs::read_link(device_path)
        .ok()
        .and_then(|target| target.file_name().map(|s| s.to_string_lossy().to_string()))
}

/// read a sysfs file as trimmed string
pub fn read_sysfs_string(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// natural sort key for interface names (e.g., eth2 < eth10)
fn natural_sort_key(s: &str) -> (String, u32) {
    let mut prefix = String::new();
    let mut num_str = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            num_str.push(c);
        } else if num_str.is_empty() {
            prefix.push(c);
        } else {
            break;
        }
    }

    let num = num_str.parse().unwrap_or(0);
    (prefix, num)
}

/// list PCI devices from /sys/bus/pci/devices
pub fn list_pci_devices() -> Result<Vec<PciDevice>> {
    let pci_path = Path::new(PCI_DEVICES);
    if !pci_path.exists() {
        return Ok(Vec::new());
    }

    let mut devices = Vec::new();

    for entry in fs::read_dir(pci_path).context("failed to read /sys/bus/pci/devices")? {
        let entry = entry?;
        let slot = entry.file_name().to_string_lossy().to_string();
        let dev_path = entry.path();

        let vendor = read_sysfs_string(&dev_path.join("vendor"));
        let device = read_sysfs_string(&dev_path.join("device"));
        let class = read_sysfs_string(&dev_path.join("class"));
        let iommu_group = dev_path
            .join("iommu_group")
            .read_link()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()));

        devices.push(PciDevice {
            slot,
            vendor,
            device,
            class,
            iommu_group,
            path: dev_path,
        });
    }

    devices.sort_by(|a, b| a.slot.cmp(&b.slot));
    Ok(devices)
}

#[derive(Debug, Clone)]
pub struct PciDevice {
    pub slot: String,
    #[allow(dead_code)]
    pub vendor: Option<String>,
    #[allow(dead_code)]
    pub device: Option<String>,
    pub class: Option<String>,
    pub iommu_group: Option<String>,
    pub path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_natural_sort_key() {
        assert!(natural_sort_key("eth2") < natural_sort_key("eth10"));
        assert!(natural_sort_key("p0") < natural_sort_key("p1"));
        assert!(natural_sort_key("mlx5_0") < natural_sort_key("mlx5_1"));
    }
}
