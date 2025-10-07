use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlatformType {
    OpenShift,
    CoreWeave,
    GKE,
    GenericKubernetes,
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
pub struct NodeInfo {
    pub name: String,
    pub rdma_capable: bool,
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
    // GKE-specific fields
    pub gke_nodepool: Option<String>,
    pub gke_machine_family: Option<String>,
    pub gke_zone: Option<String>,
    pub gke_rdma_interfaces: Vec<GkeRdmaInterface>,
    pub gke_pci_topology: Option<String>,
    pub gke_fabric_domain: Option<String>,
    pub gke_topology_block: Option<String>,
    pub gke_topology_subblock: Option<String>,
    pub gke_topology_host: Option<String>,
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
