use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sideway::ibverbs::{
    address::{GidEntry, GidType},
    device::{self, DeviceInfo},
    device_context::PortState,
};
use std::collections::HashMap;
use std::net::Ipv6Addr;
use tracing::{debug, info, warn};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Detect and configure RoCE HCAs for NCCL, NVSHMEM, and UCX"
)]
struct Args {
    /// Output format: env (shell export), json, or quiet (env vars only)
    #[arg(short, long, default_value = "env")]
    format: OutputFormat,

    /// Filter HCAs by network interface name (comma-separated)
    #[arg(short = 'i', long)]
    socket_ifname: Option<String>,

    /// Force a specific GID index (overrides auto-detection)
    #[arg(short = 'g', long)]
    gid_index: Option<u32>,

    /// Device prefix to filter (e.g., "mlx5_", "mlx4_", "bnxt_")
    #[arg(short = 'p', long, default_value = "mlx5_")]
    device_prefix: String,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    /// Shell export statements
    Env,
    /// JSON output
    Json,
    /// Quiet mode - only env var values
    Quiet,
}

#[derive(Debug, Serialize, Deserialize)]
struct RoceConfig {
    /// List of active HCA names (e.g., ["mlx5_0", "mlx5_1"])
    active_hcas: Vec<String>,
    /// HCAs to use for NCCL (after filtering)
    nccl_hcas: Vec<String>,
    /// HCAs with port numbers for UCX/NVSHMEM
    ucx_hcas: Vec<String>,
    /// Selected GID index
    gid_index: Option<u32>,
    /// GID index counts per index
    gid_index_counts: HashMap<u32, u32>,
    /// Per-HCA GID info
    hca_details: Vec<HcaDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HcaDetail {
    name: String,
    port_state: String,
    has_roce_v2: bool,
    gid_index: Option<u32>,
    gid_value: Option<String>,
    netdev: Option<String>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();

    let config = detect_roce_config(&args)?;

    match args.format {
        OutputFormat::Env => print_env_output(&config),
        OutputFormat::Json => print_json_output(&config)?,
        OutputFormat::Quiet => print_quiet_output(&config),
    }

    Ok(())
}

fn detect_roce_config(args: &Args) -> Result<RoceConfig> {
    info!("discovering active RoCE HCAs using ibverbs");

    let device_list = device::DeviceList::new()
        .context("failed to enumerate RDMA devices - ensure ibverbs is available")?;

    let mut active_hcas = Vec::new();
    let mut hca_details = Vec::new();
    let mut gid_index_counts: HashMap<u32, u32> = HashMap::new();

    // single-pass device enumeration - collect all info at once
    for device in &device_list {
        let name = device.name();

        if !name.starts_with(&args.device_prefix) {
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

        // query GID table once and extract all needed info
        let gid_entries = ctx
            .query_gid_table()
            .with_context(|| format!("failed to query GID table for {}", name))?;

        let roce_v2_gid = gid_entries
            .iter()
            .find(|gid| gid.port_num() == 1 && matches!(gid.gid_type(), GidType::RoceV2));

        let has_roce_v2 = roce_v2_gid.is_some();
        let netdev = roce_v2_gid.and_then(|gid| gid.netdev_name().ok());

        // detect GID index during this pass (avoid re-enumeration)
        let (gid_index, gid_value) = if is_active && has_roce_v2 {
            find_ipv4_gid_index(&gid_entries, &mut gid_index_counts, &name)
        } else {
            (None, None)
        };

        let detail = HcaDetail {
            name: name.to_string(),
            port_state: format!("{:?}", port_state),
            has_roce_v2,
            gid_index,
            gid_value,
            netdev,
        };

        if is_active && has_roce_v2 {
            info!("found active HCA: {}", name);
            active_hcas.push(name.to_string());
            hca_details.push(detail);
        } else {
            debug!(
                "skipping inactive or non-RoCE HCA: {} (state={:?}, roce={})",
                name, port_state, has_roce_v2
            );
            hca_details.push(detail);
        }
    }

    if active_hcas.is_empty() {
        warn!("no active RoCE HCAs found");
        return Ok(RoceConfig {
            active_hcas: vec![],
            nccl_hcas: vec![],
            ucx_hcas: vec![],
            gid_index: None,
            gid_index_counts: HashMap::new(),
            hca_details,
        });
    }

    // filter by NCCL_SOCKET_IFNAME if specified
    let nccl_hcas = if let Some(ref ifnames) = args.socket_ifname {
        filter_hcas_by_interface(&active_hcas, ifnames, &hca_details)?
    } else {
        active_hcas.clone()
    };

    // build UCX HCA list with port numbers
    let ucx_hcas: Vec<String> = active_hcas.iter().map(|h| format!("{}:1", h)).collect();

    // determine best GID index
    let gid_index = if let Some(forced_gid) = args.gid_index {
        info!("using forced GID index: {}", forced_gid);
        Some(forced_gid)
    } else {
        select_best_gid_index(&gid_index_counts)
    };

    Ok(RoceConfig {
        active_hcas,
        nccl_hcas,
        ucx_hcas,
        gid_index,
        gid_index_counts,
        hca_details,
    })
}

fn filter_hcas_by_interface(
    active_hcas: &[String],
    ifnames: &str,
    hca_details: &[HcaDetail],
) -> Result<Vec<String>> {
    info!("filtering HCAs by interface: {}", ifnames);

    // use HashSet for O(1) lookups instead of Vec contains
    let target_ifaces: std::collections::HashSet<&str> =
        ifnames.split(',').map(|s| s.trim()).collect();

    // build HashMap for O(1) HCA detail lookups
    let detail_map: HashMap<&str, &HcaDetail> =
        hca_details.iter().map(|d| (d.name.as_str(), d)).collect();

    let mut filtered = Vec::new();

    for hca in active_hcas {
        if let Some(detail) = detail_map.get(hca.as_str()) {
            if let Some(ref netdev) = detail.netdev {
                if target_ifaces.contains(netdev.as_str()) {
                    info!("HCA {} matches interface {}", hca, netdev);
                    filtered.push(hca.clone());
                }
            }
        }
    }

    if filtered.is_empty() {
        warn!("no HCAs matched NCCL_SOCKET_IFNAME, using all active HCAs");
        Ok(active_hcas.to_vec())
    } else {
        Ok(filtered)
    }
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
        .max_by_key(|(_, &count)| count)
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

fn print_env_output(config: &RoceConfig) {
    println!("# RoCE HCA Configuration");
    println!("# Generated by roce-detector");
    println!();

    if config.nccl_hcas.is_empty() {
        println!("# WARNING: No active RoCE HCAs found");
        return;
    }

    let nccl_hcas_str = config.nccl_hcas.join(",");
    let ucx_hcas_str = config.ucx_hcas.join(",");

    println!("export NCCL_IB_HCA=\"={}\"", nccl_hcas_str);
    println!("export NVSHMEM_HCA_LIST=\"{}\"", ucx_hcas_str);
    println!("export UCX_NET_DEVICES=\"{}\"", ucx_hcas_str);

    if let Some(gid_idx) = config.gid_index {
        println!();
        println!("export NCCL_IB_GID_INDEX=\"{}\"", gid_idx);
        println!("export NVSHMEM_IB_GID_INDEX=\"{}\"", gid_idx);
        println!("export UCX_IB_GID_INDEX=\"{}\"", gid_idx);
    }

    println!();
    println!("# Active HCAs: {}", config.active_hcas.join(", "));
}

fn print_json_output(config: &RoceConfig) -> Result<()> {
    let json = serde_json::to_string_pretty(config)?;
    println!("{}", json);
    Ok(())
}

fn print_quiet_output(config: &RoceConfig) {
    if config.nccl_hcas.is_empty() {
        return;
    }

    let nccl_hcas_str = config.nccl_hcas.join(",");
    let ucx_hcas_str = config.ucx_hcas.join(",");

    println!("NCCL_IB_HCA=\"={}\"", nccl_hcas_str);
    println!("NVSHMEM_HCA_LIST=\"{}\"", ucx_hcas_str);
    println!("UCX_NET_DEVICES=\"{}\"", ucx_hcas_str);

    if let Some(gid_idx) = config.gid_index {
        println!("NCCL_IB_GID_INDEX=\"{}\"", gid_idx);
        println!("NVSHMEM_IB_GID_INDEX=\"{}\"", gid_idx);
        println!("UCX_IB_GID_INDEX=\"{}\"", gid_idx);
    }
}
