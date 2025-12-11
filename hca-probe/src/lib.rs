use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sideway::ibverbs::{
    address::{GidEntry, GidType},
    device::{self, DeviceInfo},
    device_context::{Guid, LinkLayer as SidewayLinkLayer, PortState},
};
use std::collections::HashMap;
use std::net::Ipv6Addr;
use std::path::Path;

const IB_CLASS: &str = "/sys/class/infiniband";
use tracing::{debug, info, warn};

/// link layer type for RDMA devices
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LinkLayer {
    InfiniBand,
    Ethernet, // RoCE
    #[default]
    Unknown,
}

impl std::fmt::Display for LinkLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkLayer::InfiniBand => write!(f, "InfiniBand"),
            LinkLayer::Ethernet => write!(f, "Ethernet"),
            LinkLayer::Unknown => write!(f, "Unknown"),
        }
    }
}

impl From<SidewayLinkLayer> for LinkLayer {
    fn from(ll: SidewayLinkLayer) -> Self {
        match ll {
            SidewayLinkLayer::InfiniBand => LinkLayer::InfiniBand,
            SidewayLinkLayer::Ethernet => LinkLayer::Ethernet,
            SidewayLinkLayer::Unspecified => LinkLayer::Unknown,
        }
    }
}

/// format a GUID, filtering out zero values
fn format_guid(guid: Guid) -> Option<String> {
    let s = guid.to_string();
    if s == "0000:0000:0000:0000" {
        None
    } else {
        Some(s)
    }
}

// struct-of-arrays design for cache-friendly iteration
#[derive(Debug, Serialize, Deserialize)]
pub struct RoceConfig {
    names: Vec<String>,
    port_states: Vec<String>,
    link_layers: Vec<LinkLayer>,
    has_roce_v2: Vec<bool>,
    gid_indices: Vec<Option<u32>>,
    gid_values: Vec<Option<String>>,
    netdevs: Vec<Option<String>>,
    node_guids: Vec<Option<String>>,
    port_lids: Vec<Option<u16>>,
    is_vf: Vec<bool>,

    // filter criteria
    socket_ifname_filter: Option<Vec<String>>,
    forced_gid_index: Option<u32>,
    link_layer_filter: Option<LinkLayer>,
    exclude_vfs: bool,
}

// pivot to row format for reporting
#[derive(Debug, Serialize, Deserialize)]
pub struct HcaDetail {
    pub name: String,
    pub port_state: String,
    pub link_layer: LinkLayer,
    pub has_roce_v2: bool,
    pub gid_index: Option<u32>,
    pub gid_value: Option<String>,
    pub netdev: Option<String>,
    pub node_guid: Option<String>,
    pub port_lid: Option<u16>,
    pub is_vf: bool,
}

impl RoceConfig {
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    fn is_active(&self, idx: usize) -> bool {
        let port_active = self.port_states[idx] == "Active";
        match self.link_layers[idx] {
            // for RoCE, require RoCE v2 support
            LinkLayer::Ethernet => port_active && self.has_roce_v2[idx],
            // for IB, just need active port (no GID needed)
            LinkLayer::InfiniBand => port_active,
            LinkLayer::Unknown => false,
        }
    }

    fn matches_filter(&self, idx: usize) -> bool {
        let link_layer_ok = match self.link_layer_filter {
            Some(filter) => self.link_layers[idx] == filter,
            None => true,
        };
        let vf_ok = !self.exclude_vfs || !self.is_vf[idx];
        link_layer_ok && vf_ok
    }

    // compute active HCA indices (respecting link layer filter)
    fn active_indices(&self) -> Vec<usize> {
        (0..self.len())
            .filter(|&i| self.is_active(i) && self.matches_filter(i))
            .collect()
    }

    /// get active InfiniBand HCAs
    pub fn infiniband_hcas(&self) -> Vec<String> {
        (0..self.len())
            .filter(|&i| {
                self.port_states[i] == "Active" && self.link_layers[i] == LinkLayer::InfiniBand
            })
            .map(|i| self.names[i].clone())
            .collect()
    }

    /// get active RoCE HCAs
    pub fn roce_hcas(&self) -> Vec<String> {
        (0..self.len())
            .filter(|&i| {
                self.port_states[i] == "Active"
                    && self.link_layers[i] == LinkLayer::Ethernet
                    && self.has_roce_v2[i]
            })
            .map(|i| self.names[i].clone())
            .collect()
    }

    // compute NCCL HCA names after filtering
    pub fn nccl_hcas(&self) -> Vec<String> {
        let active = self.active_indices();

        let filtered = if let Some(ref filters) = self.socket_ifname_filter {
            active
                .into_iter()
                .filter(|&i| {
                    self.netdevs[i]
                        .as_ref()
                        .map(|netdev| filters.iter().any(|f| f == netdev))
                        .unwrap_or(false)
                })
                .collect()
        } else {
            active
        };

        if filtered.is_empty() && !self.active_indices().is_empty() {
            warn!("no HCAs matched filter, using all active");
            self.active_indices()
                .iter()
                .map(|&i| self.names[i].clone())
                .collect()
        } else {
            filtered.iter().map(|&i| self.names[i].clone()).collect()
        }
    }

    pub fn active_hcas(&self) -> Vec<String> {
        self.active_indices()
            .iter()
            .map(|&i| self.names[i].clone())
            .collect()
    }

    pub fn ucx_hcas(&self) -> Vec<String> {
        self.active_hcas()
            .iter()
            .map(|h| format!("{}:1", h))
            .collect()
    }

    pub fn gid_index_counts(&self) -> HashMap<u32, u32> {
        self.active_indices()
            .iter()
            .filter_map(|&i| self.gid_indices[i])
            .fold(HashMap::new(), |mut map, idx| {
                *map.entry(idx).or_insert(0) += 1;
                map
            })
    }

    pub fn selected_gid_index(&self) -> Option<u32> {
        self.forced_gid_index
            .or_else(|| select_best_gid_index(&self.gid_index_counts()))
    }

    // pivot to array-of-structs for reporting
    pub fn to_details(&self) -> Vec<HcaDetail> {
        (0..self.len())
            .filter(|&i| self.matches_filter(i))
            .map(|i| HcaDetail {
                name: self.names[i].clone(),
                port_state: self.port_states[i].clone(),
                link_layer: self.link_layers[i],
                has_roce_v2: self.has_roce_v2[i],
                gid_index: self.gid_indices[i],
                gid_value: self.gid_values[i].clone(),
                netdev: self.netdevs[i].clone(),
                node_guid: self.node_guids[i].clone(),
                port_lid: self.port_lids[i],
                is_vf: self.is_vf[i],
            })
            .collect()
    }

    /// check if any RoCE devices are present (for GID index output decision)
    pub fn has_roce_devices(&self) -> bool {
        (0..self.len()).any(|i| {
            self.link_layers[i] == LinkLayer::Ethernet
                && self.has_roce_v2[i]
                && self.port_states[i] == "Active"
        })
    }
}

/// Detect RDMA configuration with optional filtering
pub fn detect_roce_config(
    device_prefix: &str,
    socket_ifname: Option<&str>,
    forced_gid_index: Option<u32>,
) -> Result<RoceConfig> {
    detect_rdma_config(device_prefix, socket_ifname, forced_gid_index, None, false)
}

/// Detect RDMA configuration with optional link layer filter and VF exclusion
pub fn detect_rdma_config(
    device_prefix: &str,
    socket_ifname: Option<&str>,
    forced_gid_index: Option<u32>,
    link_layer_filter: Option<LinkLayer>,
    exclude_vfs: bool,
) -> Result<RoceConfig> {
    info!("discovering RDMA HCAs using ibverbs");

    let device_list = device::DeviceList::new()
        .context("failed to enumerate RDMA devices - ensure ibverbs is available")?;

    // struct-of-arrays storage
    let mut names = Vec::new();
    let mut port_states = Vec::new();
    let mut link_layers = Vec::new();
    let mut has_roce_v2 = Vec::new();
    let mut gid_indices = Vec::new();
    let mut gid_values = Vec::new();
    let mut netdevs = Vec::new();
    let mut node_guids = Vec::new();
    let mut port_lids = Vec::new();
    let mut is_vf_vec = Vec::new();

    let mut gid_index_counts: HashMap<u32, u32> = HashMap::new();

    // enumerate devices
    for device in &device_list {
        let name = device.name();

        // only filter by prefix if non-empty
        if !device_prefix.is_empty() && !name.starts_with(device_prefix) {
            continue;
        }

        debug!("checking HCA: {}", name);

        let ctx = device
            .open()
            .with_context(|| format!("failed to open device context for {}", name))?;

        let port_attr = ctx
            .query_port(1)
            .with_context(|| format!("failed to query port state for {}", name))?;

        let port_state = port_attr.port_state();
        let is_active = matches!(port_state, PortState::Active);

        // get link layer from ibverbs (not sysfs)
        let link_layer: LinkLayer = port_attr.link_layer().into();

        // detect if this is a VF (Virtual Function) - requires sysfs, no ibverbs API
        let is_vf = is_virtual_function(&name);

        // get node GUID from ibverbs (not sysfs)
        let node_guid = format_guid(device.guid());

        // get port LID for IB devices (only meaningful for InfiniBand, returns 0 for RoCE)
        let port_lid = if link_layer == LinkLayer::InfiniBand {
            let lid = port_attr.lid();
            if lid != 0 { Some(lid) } else { None }
        } else {
            None
        };

        // query GID table once and extract all needed info
        let gid_entries = ctx
            .query_gid_table()
            .with_context(|| format!("failed to query GID table for {}", name))?;

        let roce_v2_gid = gid_entries
            .iter()
            .find(|gid| gid.port_num() == 1 && matches!(gid.gid_type(), GidType::RoceV2));

        let has_roce = roce_v2_gid.is_some();
        let netdev = roce_v2_gid.and_then(|gid| gid.netdev_name().ok());

        // only find GID index for RoCE (Ethernet) devices
        let (gid_index, gid_value) = if is_active && link_layer == LinkLayer::Ethernet && has_roce {
            find_ipv4_gid_index(&gid_entries, &mut gid_index_counts, &name)
        } else {
            (None, None)
        };

        names.push(name.to_string());
        port_states.push(format!("{:?}", port_state));
        link_layers.push(link_layer);
        has_roce_v2.push(has_roce);
        gid_indices.push(gid_index);
        gid_values.push(gid_value);
        netdevs.push(netdev);
        node_guids.push(node_guid);
        port_lids.push(port_lid);
        is_vf_vec.push(is_vf);

        let vf_suffix = if is_vf { " (VF)" } else { "" };
        if is_active {
            match link_layer {
                LinkLayer::InfiniBand => {
                    info!("found active InfiniBand HCA: {}{}", name, vf_suffix);
                }
                LinkLayer::Ethernet if has_roce => {
                    info!("found active RoCE HCA: {}{}", name, vf_suffix);
                }
                _ => {
                    debug!(
                        "found HCA with no usable transport: {} (link_layer={:?}, roce={})",
                        name, link_layer, has_roce
                    );
                }
            }
        } else {
            debug!(
                "skipping inactive HCA: {} (state={:?}, link_layer={:?})",
                name, port_state, link_layer
            );
        }
    }

    let socket_ifname_filter =
        socket_ifname.map(|s| s.split(',').map(|f| f.trim().to_string()).collect());

    let config = RoceConfig {
        names,
        port_states,
        link_layers,
        has_roce_v2,
        gid_indices,
        gid_values,
        netdevs,
        node_guids,
        port_lids,
        is_vf: is_vf_vec,
        socket_ifname_filter,
        forced_gid_index,
        link_layer_filter,
        exclude_vfs,
    };

    if config.active_indices().is_empty() {
        warn!("no active RDMA HCAs found");
    }

    Ok(config)
}

/// detect if device is a Virtual Function (VF) by checking for physfn symlink
fn is_virtual_function(device_name: &str) -> bool {
    // VFs have a physfn symlink pointing to their parent PF
    let device_path = Path::new(IB_CLASS).join(device_name).join("device");
    device_path.is_symlink() && device_path.join("physfn").is_symlink()
}

/// Get list of active HCAs (convenience wrapper)
pub fn get_active_hcas(device_prefix: &str) -> Result<Vec<String>> {
    let config = detect_roce_config(device_prefix, None, None)?;
    Ok(config.active_hcas())
}

/// Get socket interface name filter (returns filtered HCA list)
pub fn get_socket_ifname(device_prefix: &str, socket_ifname: &str) -> Result<Vec<String>> {
    let config = detect_roce_config(device_prefix, Some(socket_ifname), None)?;
    Ok(config.nccl_hcas())
}

// find IPv4 GID index for a single HCA during enumeration
fn find_ipv4_gid_index(
    gid_entries: &[GidEntry],
    gid_index_counts: &mut HashMap<u32, u32>,
    name: &str,
) -> (Option<u32>, Option<String>) {
    for gid_entry in gid_entries {
        if gid_entry.port_num() != 1 || !matches!(gid_entry.gid_type(), GidType::RoceV2) {
            continue;
        }

        let gid = gid_entry.gid();
        let ipv6 = Ipv6Addr::from(gid);

        // check for IPv4-mapped IPv6 address (::ffff:a.b.c.d pattern)
        if ipv6.to_ipv4_mapped().is_some() {
            let idx = gid_entry.gid_index() as u32;
            let gid_str = format!("{}", gid);

            info!(
                "found IPv4 RoCE v2 GID for {}: index={}, gid={}",
                name, idx, gid_str
            );

            *gid_index_counts.entry(idx).or_insert(0) += 1;

            return (Some(idx), Some(gid_str));
        }
    }

    (None, None)
}

// select best GID index from collected counts
fn select_best_gid_index(gid_index_counts: &HashMap<u32, u32>) -> Option<u32> {
    if gid_index_counts.is_empty() {
        warn!("no valid IPv4 RoCE v2 GID_INDEX found on any HCA");
        return None;
    }

    // find the most common GID index
    let (mut best_gid_index, max_count) = gid_index_counts
        .iter()
        .max_by_key(|&(_, &count)| count)
        .map(|(&idx, &count)| (idx, count))
        .unwrap_or((0, 0));

    for (idx, count) in gid_index_counts {
        info!("GID_INDEX {} found on {} HCAs", idx, count);
    }

    // deterministic fallback: prefer index 3 for SR-IOV
    if gid_index_counts.len() > 1 {
        if let Some(&count_3) = gid_index_counts.get(&3) {
            if count_3 == max_count {
                info!("using deterministic fallback: GID_INDEX=3 (SR-IOV standard)");
                best_gid_index = 3;
            }
        }
    }

    info!(
        "selected GID_INDEX: {} (found on {} HCAs)",
        best_gid_index, max_count
    );

    Some(best_gid_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roce_config_empty() {
        let config = RoceConfig {
            names: vec![],
            port_states: vec![],
            link_layers: vec![],
            has_roce_v2: vec![],
            gid_indices: vec![],
            gid_values: vec![],
            netdevs: vec![],
            node_guids: vec![],
            port_lids: vec![],
            is_vf: vec![],
            socket_ifname_filter: None,
            forced_gid_index: None,
            link_layer_filter: None,
            exclude_vfs: false,
        };

        assert_eq!(config.len(), 0);
        assert!(config.is_empty());
        assert!(config.active_hcas().is_empty());
    }

    #[test]
    fn test_link_layer_display() {
        assert_eq!(format!("{}", LinkLayer::InfiniBand), "InfiniBand");
        assert_eq!(format!("{}", LinkLayer::Ethernet), "Ethernet");
        assert_eq!(format!("{}", LinkLayer::Unknown), "Unknown");
    }

    #[test]
    fn test_mixed_hca_types() {
        let config = RoceConfig {
            names: vec!["ibp0".to_string(), "mlx5_0".to_string()],
            port_states: vec!["Active".to_string(), "Active".to_string()],
            link_layers: vec![LinkLayer::InfiniBand, LinkLayer::Ethernet],
            has_roce_v2: vec![false, true],
            gid_indices: vec![None, Some(3)],
            gid_values: vec![None, Some("::ffff:10.0.0.1".to_string())],
            netdevs: vec![None, Some("eth0".to_string())],
            node_guids: vec![Some("9c63:c003:00d8:7d92".to_string()), None],
            port_lids: vec![Some(1028), None],
            is_vf: vec![false, false],
            socket_ifname_filter: None,
            forced_gid_index: None,
            link_layer_filter: None,
            exclude_vfs: false,
        };

        assert_eq!(config.len(), 2);
        assert_eq!(config.infiniband_hcas(), vec!["ibp0"]);
        assert_eq!(config.roce_hcas(), vec!["mlx5_0"]);
        assert_eq!(config.active_hcas().len(), 2);
    }

    #[test]
    fn test_vf_filtering() {
        let config = RoceConfig {
            names: vec![
                "mlx5_5".to_string(), // PF
                "mlx5_0".to_string(), // VF
                "mlx5_1".to_string(), // VF
            ],
            port_states: vec![
                "Active".to_string(),
                "Active".to_string(),
                "Active".to_string(),
            ],
            link_layers: vec![
                LinkLayer::Ethernet,
                LinkLayer::Ethernet,
                LinkLayer::Ethernet,
            ],
            has_roce_v2: vec![true, true, true],
            gid_indices: vec![Some(3), None, None],
            gid_values: vec![Some("::ffff:10.0.0.1".to_string()), None, None],
            netdevs: vec![
                Some("enp157s0np0".to_string()),
                Some("enp157s0v0".to_string()),
                Some("enp157s0v1".to_string()),
            ],
            node_guids: vec![Some("5c25:7303:0012:bfa0".to_string()), None, None],
            port_lids: vec![None, None, None],
            is_vf: vec![false, true, true],
            socket_ifname_filter: None,
            forced_gid_index: None,
            link_layer_filter: None,
            exclude_vfs: true, // exclude VFs
        };

        // with exclude_vfs=true, only PF should be active
        assert_eq!(config.active_hcas(), vec!["mlx5_5"]);
        assert_eq!(config.to_details().len(), 1);
    }
}
