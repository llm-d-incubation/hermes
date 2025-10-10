use chrono::{DateTime, Utc};
use minijinja::value::{Object, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlatformType {
    OpenShift,
    CoreWeave,
    GKE,
    GenericKubernetes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdmaCapability {
    Capable,
    NotCapable,
}

impl RdmaCapability {
    pub fn is_capable(&self) -> bool {
        matches!(self, Self::Capable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageCacheStatus {
    Cached,
    NotCached,
    Unknown,
}

impl ImageCacheStatus {
    pub fn is_cached(&self) -> bool {
        matches!(self, Self::Cached)
    }
}

impl From<Option<bool>> for ImageCacheStatus {
    fn from(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Cached,
            Some(false) => Self::NotCached,
            None => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelDetailLevel {
    Basic,
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeFilter {
    All,
    RdmaOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheMode {
    UseCache,
    SkipCache,
}

impl CacheMode {
    pub fn should_use_cache(&self) -> bool {
        matches!(self, Self::UseCache)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadSource {
    Embedded,
    Stdin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupMode {
    Cleanup,
    NoCleanup,
}

impl CleanupMode {
    pub fn should_cleanup(&self) -> bool {
        matches!(self, Self::Cleanup)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    DryRun,
    Execute,
}

impl ExecutionMode {
    pub fn is_dry_run(&self) -> bool {
        matches!(self, Self::DryRun)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuRequirement {
    Required,
    NotRequired,
}

impl GpuRequirement {
    pub fn requires_gpu(&self) -> bool {
        matches!(self, Self::Required)
    }
}

impl Object for GpuRequirement {
    fn get_value(self: &std::sync::Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "requires_gpu" => Some(Value::from(self.requires_gpu())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalHandling {
    CleanupOnSignal,
    NoCleanupOnSignal,
}

impl SignalHandling {
    pub fn should_cleanup_on_signal(&self) -> bool {
        matches!(self, Self::CleanupOnSignal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageCacheCheck {
    CheckCache,
    SkipCacheCheck,
}

impl ImageCacheCheck {
    pub fn should_check_cache(&self) -> bool {
        matches!(self, Self::CheckCache)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TopologyType {
    LeafGroup, // CoreWeave leafgroup-based
    Zone,      // Kubernetes zone-based
    Rack,      // Kubernetes rack-based
    IpRange,   // IP address range-based
    Subnet,    // Network subnet-based
    Hardware,  // Hardware/machine type-based
    GkeBlock,  // GKE topology block (rail-aligned for RDMA)
    Unknown,   // No topology detected
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyDetection {
    pub topology_type: TopologyType,
    pub detection_method: String,
    pub confidence: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PlatformSpecificData {
    Gke(Box<GkePlatformData>),
    Generic,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GkePlatformData {
    pub nodepool: Option<String>,
    pub machine_family: Option<String>,
    pub zone: Option<String>,
    pub rdma_interfaces: Vec<GkeRdmaInterface>,
    pub pci_topology: Option<String>,
    pub fabric_domain: Option<String>,
    pub topology_block: Option<String>,
    pub topology_subblock: Option<String>,
    pub topology_host: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NodeInfo {
    pub name: String,
    pub rdma_capability: RdmaCapability,
    pub rdma_type: Option<String>,
    pub rdma_resource: Option<String>,
    pub platform_type: PlatformType,
    pub topology_block: Option<String>,
    pub topology_detection: Option<TopologyDetection>,
    pub ib_speed: Option<String>,
    pub ib_fabric: Option<String>,
    pub ib_ports: Option<String>,
    pub leafgroup: Option<String>,
    pub superpod: Option<String>,
    pub neighbors: Vec<String>,
    pub mellanox_nics: Vec<MellanoxNic>,
    pub node_labels: HashMap<String, String>,
    pub gpu_count: Option<u32>,
    pub gpu_type: Option<String>,
    pub gpu_allocatable: Option<u32>,
    pub gpu_allocated: Option<u32>,
    // platform-specific data
    pub platform_data: PlatformSpecificData,
    // image cache tracking
    pub image_cache_status: ImageCacheStatus,
    pub image_cache_checked_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MellanoxNic {
    pub part_number: Option<String>,
    pub firmware: Option<String>,
    pub interface: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GkeRdmaInterface {
    pub network_name: String,
    pub pci_address: String,
    pub birth_name: String,
    pub ip_address: String,
    pub subnet: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SriovNetworkInfo {
    pub name: String,
    pub namespace: String,
    pub resource_name: String,
    pub vlan: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClusterReport {
    pub total_nodes: usize,
    pub rdma_nodes: usize,
    pub platform_type: PlatformType,
    pub topology_detection: Option<TopologyDetection>,
    pub rdma_types: Vec<String>,
    pub topology_blocks: HashMap<String, usize>,
    pub topology_gpu_counts: HashMap<String, u32>,
    pub ib_fabrics: Vec<String>,
    pub superpods: Vec<String>,
    pub leafgroups: Vec<String>,
    pub sriov_networks: Vec<SriovNetworkInfo>,
    pub nodes: Vec<NodeInfo>,
    pub gpu_nodes: usize,
    pub gpu_types: Vec<String>,
    pub total_gpus: u32,
    // image cache configuration
    pub image_checked: Option<String>,
    pub cache_check_timestamp: Option<DateTime<Utc>>,
}

impl std::fmt::Display for PlatformType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformType::OpenShift => write!(f, "OpenShift"),
            PlatformType::CoreWeave => write!(f, "CoreWeave"),
            PlatformType::GKE => write!(f, "Google Kubernetes Engine (GKE)"),
            PlatformType::GenericKubernetes => write!(f, "Generic Kubernetes"),
        }
    }
}

impl std::fmt::Display for TopologyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyType::LeafGroup => write!(f, "Leaf Group"),
            TopologyType::Zone => write!(f, "Zone"),
            TopologyType::Rack => write!(f, "Rack"),
            TopologyType::IpRange => write!(f, "IP Range"),
            TopologyType::Subnet => write!(f, "Subnet"),
            TopologyType::Hardware => write!(f, "Hardware"),
            TopologyType::GkeBlock => write!(f, "GKE Block"),
            TopologyType::Unknown => write!(f, "Unknown"),
        }
    }
}
