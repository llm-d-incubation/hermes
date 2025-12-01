use chrono::{DateTime, Utc};
use minijinja::value::{Object, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// define RoceConfig and HcaDetail types inline to avoid platform-specific dependencies
// these match the JSON output from roce-detector binary (JsonOutput struct in main.rs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoceConfig {
    pub active_hcas: Vec<String>,
    pub nccl_hcas: Vec<String>,
    pub ucx_hcas: Vec<String>,
    pub gid_index: Option<u32>,
    pub gid_index_counts: HashMap<u32, u32>,
    pub hca_details: Vec<HcaDetail>,

    // namespace-aware detection (Phase 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_configs: Option<Vec<NamespaceRoceConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid_mismatch_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_pods: Option<Vec<String>>,
}

impl RoceConfig {
    pub fn active_hcas(&self) -> Vec<String> {
        self.active_hcas.clone()
    }

    pub fn to_details(&self) -> Vec<HcaDetail> {
        self.hca_details.clone()
    }

    /// check if there are any namespace-specific configurations
    pub fn has_namespace_configs(&self) -> bool {
        self.namespace_configs
            .as_ref()
            .map(|configs| !configs.is_empty())
            .unwrap_or(false)
    }

    /// get all namespace configurations
    pub fn get_namespace_configs(&self) -> Vec<NamespaceRoceConfig> {
        self.namespace_configs.clone().unwrap_or_default()
    }

    /// detect if there are GID index mismatches across namespaces
    pub fn has_gid_mismatch(&self) -> bool {
        if let Some(configs) = &self.namespace_configs {
            let gid_indices: Vec<u32> = configs.iter().filter_map(|c| c.gid_index).collect();

            if gid_indices.len() > 1 {
                let first = gid_indices[0];
                return gid_indices.iter().any(|&idx| idx != first);
            }
        }
        false
    }

    /// get list of pods affected by GID mismatches (if any)
    pub fn affected_pods(&self) -> Vec<String> {
        if !self.has_gid_mismatch() {
            return vec![];
        }

        self.namespace_configs
            .as_ref()
            .map(|configs| {
                configs
                    .iter()
                    .filter(|c| c.namespace_type == NamespaceType::Pod)
                    .filter_map(|c| c.pod_name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// RoCE configuration for a specific network namespace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceRoceConfig {
    pub namespace_type: NamespaceType,
    pub namespace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub active_hcas: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid_index: Option<u32>,
    pub gid_index_counts: HashMap<u32, u32>,
    pub hca_details: Vec<HcaDetail>,
}

impl NamespaceRoceConfig {
    /// get total number of active HCAs in this namespace
    pub fn active_hca_count(&self) -> usize {
        self.active_hcas.len()
    }

    /// check if this namespace has any active HCAs
    pub fn has_active_hcas(&self) -> bool {
        !self.active_hcas.is_empty()
    }

    /// get a display name for this namespace
    pub fn display_name(&self) -> String {
        match &self.namespace_type {
            NamespaceType::Host => "host".to_string(),
            NamespaceType::Pod => self
                .pod_name
                .clone()
                .unwrap_or_else(|| format!("pod:{}", self.namespace_id)),
            NamespaceType::NetworkNamespace => {
                format!("netns:{}", self.namespace_id)
            }
        }
    }
}

/// Type of network namespace where RoCE detection was performed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NamespaceType {
    Host,
    Pod,
    NetworkNamespace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HcaDetail {
    pub name: String,
    pub port_state: String,
    pub has_roce_v2: bool,
    pub gid_index: Option<u32>,
    pub gid_value: Option<String>,
    pub netdev: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TopologyType {
    LeafGroup, // CoreWeave leafgroup-based
    Zone,      // Kubernetes zone-based
    Rack,      // Kubernetes rack-based
    IpRange,   // IP address range-based
    Subnet,    // Network subnet-based
    Hardware,  // Hardware/machine type-based
    GkeBlock,  // GKE topology block (rail-aligned for RDMA)
    Custom,    // Custom CEL rule-based
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
    // resource usage tracking
    pub cpu_allocatable: Option<String>,
    pub cpu_allocated: Option<String>,
    pub memory_allocatable: Option<String>,
    pub memory_allocated: Option<String>,
    // sriov device tracking
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub sriov_resources: HashMap<String, String>,
    // platform-specific data
    pub platform_data: PlatformSpecificData,
    // image cache tracking
    pub image_cache_status: ImageCacheStatus,
    pub image_cache_checked_at: Option<DateTime<Utc>>,
    // topology rule evaluation errors (for aggregated logging)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology_rule_error: Option<String>,
    // roce configuration (only populated with scan-roce command)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roce_config: Option<RoceConfig>,
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
    pub api_server_url: String,
    pub topology_detection: Option<TopologyDetection>,
    pub rdma_types: Vec<String>,
    pub topology_blocks: HashMap<String, usize>,
    pub topology_gpu_counts: HashMap<String, u32>,
    pub ib_fabrics: Vec<String>,
    pub superpods: Vec<String>,
    pub leafgroups: Vec<String>,
    pub sriov_networks: Vec<SriovNetworkInfo>,
    pub nvidia_network_operator_resources: Option<Vec<String>>,
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
            TopologyType::Custom => write!(f, "Custom"),
            TopologyType::Unknown => write!(f, "Unknown"),
        }
    }
}
