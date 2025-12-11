use anyhow::{Context, Result};
use argh::FromArgs;
use hca_probe::{HcaDetail, LinkLayer, RoceConfig, detect_rdma_config};
use nix::sched::{CloneFlags, setns};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use tracing::info;

mod iface_hca;
mod iface_ip;
mod iommu_acs;
mod sysfs;
mod vf_map;

/// Detect and configure RDMA HCAs (InfiniBand and RoCE) for NCCL, NVSHMEM, and UCX
#[derive(FromArgs, Debug)]
struct Cli {
    #[argh(subcommand)]
    command: Commands,
}

#[derive(FromArgs, Debug)]
#[argh(subcommand)]
enum Commands {
    Detect(DetectCmd),
    IfaceHca(IfaceHcaCmd),
    VfMap(VfMapCmd),
    IommuAcs(IommuAcsCmd),
    IfaceIp(IfaceIpCmd),
}

/// Detect RDMA HCAs and output configuration
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "detect")]
struct DetectCmd {
    /// output format: env, json, or quiet
    #[argh(option, short = 'f', default = "DetectOutputFormat::Env")]
    format: DetectOutputFormat,

    /// filter HCAs by network interface name (comma-separated)
    #[argh(option, short = 'i')]
    socket_ifname: Option<String>,

    /// force a specific GID index (overrides auto-detection)
    #[argh(option, short = 'g')]
    gid_index: Option<u32>,

    /// device prefix to filter (e.g., "mlx5_", "mlx4_", "bnxt_")
    #[argh(option, short = 'p', default = "String::new()")]
    device_prefix: String,

    /// filter by link layer: ib, roce, or all
    #[argh(option, short = 'l', default = "LinkLayerFilter::All")]
    link_layer: LinkLayerFilter,

    /// exclude SR-IOV Virtual Functions (VFs)
    #[argh(switch)]
    no_vf: bool,

    /// enter network namespace of specific PID before detection
    #[argh(option)]
    namespace_pid: Option<u32>,

    /// namespace identifier for output correlation
    #[argh(option)]
    namespace_id: Option<String>,
}

/// Map network interfaces to InfiniBand HCAs
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "iface-hca")]
struct IfaceHcaCmd {
    /// output format: table or json
    #[argh(option, short = 'f', default = "OutputFormat::Table")]
    format: OutputFormat,
}

/// Map SR-IOV Virtual Functions to Physical Functions
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "vf-map")]
struct VfMapCmd {
    /// output format: table or json
    #[argh(option, short = 'f', default = "OutputFormat::Table")]
    format: OutputFormat,
}

/// Check IOMMU and PCI ACS configuration
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "iommu-acs")]
struct IommuAcsCmd {
    /// output format: table or json
    #[argh(option, short = 'f', default = "OutputFormat::Table")]
    format: OutputFormat,
}

/// List network interfaces with IP addresses
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "iface-ip")]
struct IfaceIpCmd {
    /// output format: table or json
    #[argh(option, short = 'f', default = "OutputFormat::Table")]
    format: OutputFormat,
}

#[derive(Debug, Clone)]
enum LinkLayerFilter {
    Ib,
    Roce,
    All,
}

impl std::str::FromStr for LinkLayerFilter {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ib" => Ok(Self::Ib),
            "roce" => Ok(Self::Roce),
            "all" => Ok(Self::All),
            _ => Err(format!(
                "invalid link layer: {} (expected: ib, roce, all)",
                s
            )),
        }
    }
}

impl Default for LinkLayerFilter {
    fn default() -> Self {
        Self::All
    }
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

#[derive(Debug, Clone)]
pub enum DetectOutputFormat {
    Env,
    Json,
    Quiet,
}

impl std::str::FromStr for DetectOutputFormat {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "env" => Ok(Self::Env),
            "json" => Ok(Self::Json),
            "quiet" => Ok(Self::Quiet),
            _ => Err(format!(
                "invalid format: {} (expected: env, json, quiet)",
                s
            )),
        }
    }
}

impl Default for DetectOutputFormat {
    fn default() -> Self {
        Self::Env
    }
}

#[derive(Debug, Clone)]
pub enum OutputFormat {
    Table,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            _ => Err(format!("invalid format: {} (expected: table, json)", s)),
        }
    }
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Table
    }
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

    let cli: Cli = argh::from_env();

    match cli.command {
        Commands::Detect(cmd) => run_detect(
            cmd.format,
            cmd.socket_ifname,
            cmd.gid_index,
            cmd.device_prefix,
            cmd.link_layer,
            cmd.no_vf,
            cmd.namespace_pid,
            cmd.namespace_id,
        ),
        Commands::IfaceHca(cmd) => iface_hca::run(cmd.format),
        Commands::VfMap(cmd) => vf_map::run(cmd.format),
        Commands::IommuAcs(cmd) => iommu_acs::run(cmd.format),
        Commands::IfaceIp(cmd) => iface_ip::run(cmd.format),
    }
}

fn run_detect(
    format: DetectOutputFormat,
    socket_ifname: Option<String>,
    gid_index: Option<u32>,
    device_prefix: String,
    link_layer: LinkLayerFilter,
    no_vf: bool,
    namespace_pid: Option<u32>,
    namespace_id: Option<String>,
) -> Result<()> {
    if let Some(pid) = namespace_pid {
        enter_network_namespace(pid)?;
    }

    let config = detect_rdma_config(
        &device_prefix,
        socket_ifname.as_deref(),
        gid_index,
        link_layer.to_link_layer(),
        no_vf,
    )?;

    match format {
        DetectOutputFormat::Env => print_env_output(&config),
        DetectOutputFormat::Json => print_json_output(&config, namespace_id, namespace_pid)?,
        DetectOutputFormat::Quiet => print_quiet_output(&config),
    }

    Ok(())
}

fn print_env_output(config: &RoceConfig) {
    println!("# RDMA HCA Configuration");
    println!("# Generated by hca-probe");
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
