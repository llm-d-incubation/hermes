//! List network interfaces with IP addresses and physical backing

use crate::OutputFormat;
use crate::sysfs;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Serialize)]
struct IfaceIpEntry {
    logical_interface: String,
    physical_port: Option<String>,
    operstate: String,
    ip_addresses: Vec<String>,
}

#[derive(Debug, Serialize)]
struct IfaceIpOutput {
    interfaces: Vec<IfaceIpEntry>,
}

pub fn run(format: OutputFormat) -> Result<()> {
    let output = collect_interfaces()?;

    match format {
        OutputFormat::Table => print_table(&output),
        OutputFormat::Json => print_json(&output)?,
    }

    Ok(())
}

fn collect_interfaces() -> Result<IfaceIpOutput> {
    // build MAC -> physical interface name mapping
    let interfaces = sysfs::list_net_interfaces()?;
    let mut mac_to_phys: HashMap<String, String> = HashMap::new();

    for iface in &interfaces {
        if iface.has_device {
            if let Some(ref mac) = iface.mac_address {
                mac_to_phys.insert(mac.clone(), iface.name.clone());
            }
        }
    }

    // get IPs for each interface from /proc/net/fib_trie is complex,
    // so we parse /sys/class/net/*/address and /proc/net/if_inet6 + read inet addrs
    let ip_map = collect_ip_addresses()?;

    let mut entries = Vec::new();

    for iface in &interfaces {
        let ips = ip_map.get(&iface.name).cloned().unwrap_or_default();

        // skip interfaces without IPs (unless they're physical)
        if ips.is_empty() && !iface.has_device {
            continue;
        }

        // find physical backing port by MAC match
        let physical_port = if iface.has_device {
            // it IS the physical port
            None
        } else if let Some(ref mac) = iface.mac_address {
            // find physical interface with same MAC (but different name)
            mac_to_phys.get(mac).filter(|&p| p != &iface.name).cloned()
        } else {
            None
        };

        let operstate = iface
            .operstate
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // only include interfaces with IPs
        if !ips.is_empty() {
            entries.push(IfaceIpEntry {
                logical_interface: iface.name.clone(),
                physical_port,
                operstate,
                ip_addresses: ips,
            });
        }
    }

    // sort by interface name
    entries.sort_by(|a, b| a.logical_interface.cmp(&b.logical_interface));

    Ok(IfaceIpOutput {
        interfaces: entries,
    })
}

/// Collect IP addresses for all interfaces from /proc filesystem
fn collect_ip_addresses() -> Result<HashMap<String, Vec<String>>> {
    let mut result: HashMap<String, Vec<String>> = HashMap::new();

    // IPv4: parse /proc/net/fib_trie is complex, use interface-specific approach
    // read from /sys/class/net/*/device or use alternative

    // simpler: read /proc/net/route for interfaces, then get IPs via reading addr files
    // but easiest pure-rust: enumerate interfaces and read their addresses

    // approach: read /proc/net/fib_trie or parse individual interface config
    // actually simplest: use netlink via raw socket or read /proc/net files

    // for now, use /proc/net/if_inet6 for IPv6 and scan for IPv4 via /proc/net/route + arp
    // but that's incomplete...

    // practical approach: read from /sys/class/net/X/address gives MAC only
    // IPs are in /proc/net/fib_trie but parsing is complex

    // use nix crate's getifaddrs which wraps libc getifaddrs()
    collect_ips_via_getifaddrs(&mut result)?;

    Ok(result)
}

/// Use getifaddrs() to enumerate all interface addresses
fn collect_ips_via_getifaddrs(result: &mut HashMap<String, Vec<String>>) -> Result<()> {
    use nix::ifaddrs::getifaddrs;

    let addrs = getifaddrs().context("failed to get interface addresses")?;

    for ifaddr in addrs {
        let iface_name = ifaddr.interface_name.clone();

        if let Some(addr) = ifaddr.address {
            let ip_str = if let Some(sockaddr_in) = addr.as_sockaddr_in() {
                let ip = Ipv4Addr::from(sockaddr_in.ip());
                // skip localhost and link-local
                if ip.is_loopback() || ip.is_link_local() {
                    continue;
                }
                format!("{}", ip)
            } else if let Some(sockaddr_in6) = addr.as_sockaddr_in6() {
                let ip = sockaddr_in6.ip();
                // skip localhost and link-local
                if ip.is_loopback() || is_link_local_v6(&ip) {
                    continue;
                }
                format!("{}", ip)
            } else {
                continue;
            };

            result.entry(iface_name).or_default().push(ip_str);
        }
    }

    // deduplicate IPs per interface
    for ips in result.values_mut() {
        ips.sort();
        ips.dedup();
    }

    Ok(())
}

fn is_link_local_v6(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] & 0xffc0) == 0xfe80
}

fn print_table(output: &IfaceIpOutput) {
    println!(
        "{:<18} | {:<12} | {:<10} | {}",
        "Logical Interface", "PHY Port", "Status", "IP Address"
    );
    println!("{}", "=".repeat(80));

    if output.interfaces.is_empty() {
        println!("(No interfaces with IP addresses found)");
        return;
    }

    for entry in &output.interfaces {
        let phys = entry.physical_port.as_deref().unwrap_or(
            if entry.logical_interface.starts_with("eth")
                || entry.logical_interface.starts_with("en")
                || entry.logical_interface.starts_with("p")
            {
                "(Physical)"
            } else {
                "(Virtual)"
            },
        );

        let ips_str = entry.ip_addresses.join(", ");
        println!(
            "{:<18} | {:<12} | {:<10} | {}",
            entry.logical_interface, phys, entry.operstate, ips_str
        );
    }
}

fn print_json(output: &IfaceIpOutput) -> Result<()> {
    let json = serde_json::to_string_pretty(output)?;
    println!("{}", json);
    Ok(())
}
