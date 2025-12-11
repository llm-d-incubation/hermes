//! Map network interfaces to InfiniBand HCAs
//! Uses sideway/ibverbs as authoritative source for HCA info

use crate::OutputFormat;
use crate::sysfs;
use anyhow::{Context, Result};
use hca_probe::LinkLayer;
use serde::Serialize;
use sideway::ibverbs::device::{self, DeviceInfo};
use sideway::ibverbs::device_context::LinkLayer as SidewayLinkLayer;
use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Debug, Serialize)]
struct IfaceHcaMapping {
    interface: String,
    hca: Option<String>,
    is_vf: bool,
    port_state: Option<String>,
    link_layer: Option<String>,
}

#[derive(Debug, Serialize)]
struct InfiniBandHca {
    name: String,
    port_state: String,
    node_guid: Option<String>,
    port_lid: Option<u16>,
}

#[derive(Debug, Serialize)]
struct IfaceHcaOutput {
    mappings: Vec<IfaceHcaMapping>,
    infiniband_hcas: Vec<InfiniBandHca>,
}

pub fn run(format: OutputFormat) -> Result<()> {
    let output = collect_mappings()?;

    match format {
        OutputFormat::Table => print_table(&output),
        OutputFormat::Json => print_json(&output)?,
    }

    Ok(())
}

/// info collected per-HCA from ibverbs
struct HcaInfo {
    port_state: String,
    link_layer: LinkLayer,
    node_guid: Option<String>,
}

fn collect_mappings() -> Result<IfaceHcaOutput> {
    // get authoritative HCA info from ibverbs
    let device_list = device::DeviceList::new()
        .context("failed to enumerate RDMA devices - ensure ibverbs is available")?;

    // build netdev -> (HCA name, port state, link layer) mapping from ibverbs
    let mut netdev_to_hca: HashMap<String, (String, String, LinkLayer)> = HashMap::new();
    let mut hca_info_map: HashMap<String, HcaInfo> = HashMap::new();

    for device in &device_list {
        let name = device.name();

        let ctx = match device.open() {
            Ok(c) => c,
            Err(_) => continue,
        };

        let port_attr = match ctx.query_port(1) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let port_state = format!("{:?}", port_attr.port_state());

        // get link layer from ibverbs (not sysfs)
        let link_layer: LinkLayer = convert_link_layer(port_attr.link_layer());

        // get node GUID from ibverbs (not sysfs)
        let node_guid = format_guid(device.guid());

        hca_info_map.insert(
            name.to_string(),
            HcaInfo {
                port_state: port_state.clone(),
                link_layer,
                node_guid,
            },
        );

        // get netdevs from GID table (only for Ethernet/RoCE devices)
        if let Ok(gid_entries) = ctx.query_gid_table() {
            for gid in &gid_entries {
                if gid.port_num() == 1 {
                    if let Ok(netdev) = gid.netdev_name() {
                        netdev_to_hca.insert(
                            netdev.clone(),
                            (name.to_string(), port_state.clone(), link_layer),
                        );
                    }
                }
            }
        }
    }

    // get VF info from sysfs (ibverbs doesn't expose this)
    let interfaces = sysfs::list_net_interfaces()?;
    let vf_set: HashSet<String> = interfaces
        .iter()
        .filter(|i| i.is_vf)
        .map(|i| i.name.clone())
        .collect();

    let mut mappings = Vec::new();
    let mut matched_hcas: HashSet<String> = HashSet::new();

    // for each interface that has an HCA
    for (netdev, (hca_name, port_state, link_layer)) in &netdev_to_hca {
        let is_vf = vf_set.contains(netdev.as_str());
        matched_hcas.insert(hca_name.clone());

        mappings.push(IfaceHcaMapping {
            interface: netdev.clone(),
            hca: Some(hca_name.clone()),
            is_vf,
            port_state: Some(port_state.clone()),
            link_layer: Some(format!("{}", link_layer)),
        });
    }

    // sort by interface name
    mappings.sort_by(|a, b| natural_sort_cmp(&a.interface, &b.interface));

    // find InfiniBand HCAs (these don't have netdev mappings, that's normal)
    let infiniband_hcas: Vec<InfiniBandHca> = hca_info_map
        .iter()
        .filter(|(name, info)| {
            !matched_hcas.contains(*name) && info.link_layer == LinkLayer::InfiniBand
        })
        .map(|(name, info)| InfiniBandHca {
            name: name.clone(),
            port_state: info.port_state.clone(),
            node_guid: info.node_guid.clone(),
            // LID requires sysfs - sideway doesn't expose PortAttr.lid
            port_lid: get_port_lid(name, 1),
        })
        .collect();

    Ok(IfaceHcaOutput {
        mappings,
        infiniband_hcas,
    })
}

fn convert_link_layer(ll: SidewayLinkLayer) -> LinkLayer {
    match ll {
        SidewayLinkLayer::InfiniBand => LinkLayer::InfiniBand,
        SidewayLinkLayer::Ethernet => LinkLayer::Ethernet,
        SidewayLinkLayer::Unspecified => LinkLayer::Unknown,
    }
}

fn format_guid(guid: sideway::ibverbs::device_context::Guid) -> Option<String> {
    let s = guid.to_string();
    if s == "0000:0000:0000:0000" {
        None
    } else {
        Some(s)
    }
}

/// read port LID from sysfs (for InfiniBand)
/// NOTE: libibverbs exposes this via ibv_port_attr.lid, but sideway's PortAttr
/// wrapper doesn't expose it. consider upstreaming a PR to sideway.
fn get_port_lid(device_name: &str, port: u8) -> Option<u16> {
    let path = format!("/sys/class/infiniband/{}/ports/{}/lid", device_name, port);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| {
            let trimmed = s.trim();
            if trimmed.starts_with("0x") {
                u16::from_str_radix(&trimmed[2..], 16).ok()
            } else {
                trimmed.parse().ok()
            }
        })
        .filter(|&lid| lid != 0)
}

fn natural_sort_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    natural_sort_key(a).cmp(&natural_sort_key(b))
}

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

fn print_table(output: &IfaceHcaOutput) {
    println!(
        "{:<15} | {:<12} | {:<10} | {:<10} | {}",
        "Interface", "HCA", "State", "Link Layer", "Notes"
    );
    println!("{}", "=".repeat(70));

    if output.mappings.is_empty() && output.infiniband_hcas.is_empty() {
        println!("(No interfaces or HCAs found)");
        return;
    }

    for m in &output.mappings {
        let hca_str = m.hca.as_deref().unwrap_or("-");
        let state_str = m.port_state.as_deref().unwrap_or("-");
        let link_layer_str = m.link_layer.as_deref().unwrap_or("-");
        let notes = if m.is_vf { "*VF" } else { "" };
        println!(
            "{:<15} | {:<12} | {:<10} | {:<10} | {}",
            m.interface, hca_str, state_str, link_layer_str, notes
        );
    }

    if !output.infiniband_hcas.is_empty() {
        println!();
        println!("InfiniBand HCAs (no network interface - uses LID addressing):");
        println!(
            "  {:<12} | {:<10} | {:<20} | {}",
            "HCA", "State", "Node GUID", "LID"
        );
        println!("  {}", "-".repeat(55));
        for h in &output.infiniband_hcas {
            let guid_str = h.node_guid.as_deref().unwrap_or("-");
            let lid_str = h
                .port_lid
                .map(|l| l.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {:<12} | {:<10} | {:<20} | {}",
                h.name, h.port_state, guid_str, lid_str
            );
        }
    }
}

fn print_json(output: &IfaceHcaOutput) -> Result<()> {
    let json = serde_json::to_string_pretty(output)?;
    println!("{}", json);
    Ok(())
}
