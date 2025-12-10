use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nix::sched::{CloneFlags, setns};
use roce_detector::{HcaDetail, LinkLayer, RoceConfig, detect_rdma_config};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use tracing::info;

mod iface_hca;
mod iface_ip;
mod iommu_acs;
mod sysfs;
mod vf_map;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Detect and configure RoCE HCAs for NCCL, NVSHMEM, and UCX"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // top-level args for backwards compat (when no subcommand given)
    #[command(flatten)]
    detect_args: DetectArgs,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Detect RoCE HCAs and output configuration (default)
    Detect(DetectArgs),

    /// Map network interfaces to InfiniBand HCAs
    IfaceHca {
        #[command(flatten)]
        common: CommonArgs,
    },

    /// Map SR-IOV Virtual Functions to Physical Functions
    VfMap {
        #[command(flatten)]
        common: CommonArgs,
    },

    /// Check IOMMU and PCI ACS configuration
    IommuAcs {
        #[command(flatten)]
        common: CommonArgs,
    },

    /// List network interfaces with IP addresses
    IfaceIp {
        #[command(flatten)]
        common: CommonArgs,
    },
}

#[derive(Parser, Debug, Clone)]
struct DetectArgs {
    /// Output format: env (shell export), json, or quiet (env vars only)
    #[arg(short, long, default_value = "env")]
    format: DetectOutputFormat,

    /// Filter HCAs by network interface name (comma-separated)
    #[arg(short = 'i', long)]
    socket_ifname: Option<String>,

    /// Force a specific GID index (overrides auto-detection)
    #[arg(short = 'g', long)]
    gid_index: Option<u32>,

    /// Device prefix to filter (e.g., "mlx5_", "mlx4_", "bnxt_"). Empty = all devices
    #[arg(short = 'p', long, default_value = "")]
    device_prefix: String,

    /// Filter by link layer type: ib (InfiniBand), roce (Ethernet/RoCE), or all
    #[arg(short = 'l', long, default_value = "all")]
    link_layer: LinkLayerFilter,

    /// Exclude SR-IOV Virtual Functions (VFs), showing only Physical Functions (PFs)
    #[arg(long)]
    no_vf: bool,

    /// Enter network namespace of specific PID before detection
    #[arg(long)]
    namespace_pid: Option<u32>,

    /// Namespace identifier for output correlation
    #[arg(long)]
    namespace_id: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum LinkLayerFilter {
    /// InfiniBand only
    Ib,
    /// RoCE (Ethernet) only
    Roce,
    /// All RDMA devices
    All,
}

impl LinkLayerFilter {
    fn to_link_layer(&self) -> Option<LinkLayer> {
        match self {
            LinkLayerFilter::Ib => Some(LinkLayer::InfiniBand),
            LinkLayerFilter::Roce => Some(LinkLayer::Ethernet),
            LinkLayerFilter::All => None,
        }
    }
}

#[derive(Parser, Debug, Clone)]
struct CommonArgs {
    /// Output format
    #[arg(short, long, default_value = "table")]
    format: OutputFormat,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum DetectOutputFormat {
    /// Shell export statements
    Env,
    /// JSON output
    Json,
    /// Quiet mode - only env var values
    Quiet,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    /// Human-readable table
    Table,
    /// JSON output
    Json,
}

fn enter_network_namespace(pid: u32) -> Result<()> {
    let netns_path = format!("/proc/{}/ns/net", pid);
    let netns_file = File::open(&netns_path)
        .context(format!("Failed to open network namespace: {}", netns_path))?;

    setns(netns_file, CloneFlags::CLONE_NEWNET).context("Failed to enter network namespace")?;

    info!("Entered network namespace for PID {}", pid);
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Detect(args)) => run_detect(args),
        None => run_detect(cli.detect_args),
        Some(Commands::IfaceHca { common }) => iface_hca::run(common.format),
        Some(Commands::VfMap { common }) => vf_map::run(common.format),
        Some(Commands::IommuAcs { common }) => iommu_acs::run(common.format),
        Some(Commands::IfaceIp { common }) => iface_ip::run(common.format),
    }
}

fn run_detect(args: DetectArgs) -> Result<()> {
    if let Some(pid) = args.namespace_pid {
        enter_network_namespace(pid)?;
    }

    let config = detect_rdma_config(
        &args.device_prefix,
        args.socket_ifname.as_deref(),
        args.gid_index,
        args.link_layer.to_link_layer(),
        args.no_vf,
    )?;

    match args.format {
        DetectOutputFormat::Env => print_env_output(&config),
        DetectOutputFormat::Json => {
            print_json_output(&config, args.namespace_id, args.namespace_pid)?
        }
        DetectOutputFormat::Quiet => print_quiet_output(&config),
    }

    Ok(())
}

fn print_env_output(config: &RoceConfig) {
    println!("# RDMA HCA Configuration");
    println!("# Generated by roce-detector");
    println!();

    let nccl_hcas = config.nccl_hcas();
    if nccl_hcas.is_empty() {
        println!("# WARNING: No active RDMA HCAs found");
        return;
    }

    let nccl_hcas_str = nccl_hcas.join(",");
    let ucx_hcas_str = config.ucx_hcas().join(",");

    println!("export NCCL_IB_HCA=\"={}\"", nccl_hcas_str);
    println!("export NVSHMEM_HCA_LIST=\"{}\"", ucx_hcas_str);
    println!("export UCX_NET_DEVICES=\"{}\"", ucx_hcas_str);

    // GID index only needed for RoCE (Ethernet) devices
    if config.has_roce_devices() {
        if let Some(gid_idx) = config.selected_gid_index() {
            println!();
            println!("export NCCL_IB_GID_INDEX=\"{}\"", gid_idx);
            println!("export NVSHMEM_IB_GID_INDEX=\"{}\"", gid_idx);
            println!("export UCX_IB_GID_INDEX=\"{}\"", gid_idx);
        }
    }

    println!();

    // show breakdown by type
    let ib_hcas = config.infiniband_hcas();
    let roce_hcas = config.roce_hcas();

    if !ib_hcas.is_empty() {
        println!("# InfiniBand HCAs: {}", ib_hcas.join(", "));
    }
    if !roce_hcas.is_empty() {
        println!("# RoCE HCAs: {}", roce_hcas.join(", "));
    }
    println!("# Active HCAs: {}", config.active_hcas().join(", "));
}

fn print_json_output(
    config: &RoceConfig,
    namespace_id: Option<String>,
    namespace_pid: Option<u32>,
) -> Result<()> {
    #[derive(Serialize)]
    struct JsonOutput {
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace_pid: Option<u32>,
        infiniband_hcas: Vec<String>,
        roce_hcas: Vec<String>,
        active_hcas: Vec<String>,
        nccl_hcas: Vec<String>,
        ucx_hcas: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        gid_index: Option<u32>,
        gid_index_counts: HashMap<u32, u32>,
        hca_details: Vec<HcaDetail>,
    }

    let output = JsonOutput {
        namespace_id,
        namespace_pid,
        infiniband_hcas: config.infiniband_hcas(),
        roce_hcas: config.roce_hcas(),
        active_hcas: config.active_hcas(),
        nccl_hcas: config.nccl_hcas(),
        ucx_hcas: config.ucx_hcas(),
        gid_index: if config.has_roce_devices() {
            config.selected_gid_index()
        } else {
            None
        },
        gid_index_counts: config.gid_index_counts(),
        hca_details: config.to_details(),
    };

    let json = serde_json::to_string_pretty(&output)?;
    println!("{}", json);
    Ok(())
}

fn print_quiet_output(config: &RoceConfig) {
    let nccl_hcas = config.nccl_hcas();
    if nccl_hcas.is_empty() {
        return;
    }

    let nccl_hcas_str = nccl_hcas.join(",");
    let ucx_hcas_str = config.ucx_hcas().join(",");

    println!("NCCL_IB_HCA=\"={}\"", nccl_hcas_str);
    println!("NVSHMEM_HCA_LIST=\"{}\"", ucx_hcas_str);
    println!("UCX_NET_DEVICES=\"{}\"", ucx_hcas_str);

    // GID index only needed for RoCE devices
    if config.has_roce_devices() {
        if let Some(gid_idx) = config.selected_gid_index() {
            println!("NCCL_IB_GID_INDEX=\"{}\"", gid_idx);
            println!("NVSHMEM_IB_GID_INDEX=\"{}\"", gid_idx);
            println!("UCX_IB_GID_INDEX=\"{}\"", gid_idx);
        }
    }
}
