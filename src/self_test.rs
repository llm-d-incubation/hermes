use anyhow::Result;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Api, Client, api::ListParams};
use minijinja::value::{Object, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::Mutex;
use tracing::info;

use crate::analyzer::ClusterAnalyzer;
use crate::crds::sriovnetworks::SriovNetwork;
use crate::models::{
    CleanupMode, ClusterReport, ExecutionMode, GpuRequirement, ImageCacheCheck, LabelDetailLevel,
    NodeInfo, PlatformType, SignalHandling, WorkloadSource,
};
use crate::platforms::*;
use crate::topology_selector::get_topology_selector;
use crate::workloads;

/// Configuration for self-test execution
#[derive(Debug, Clone, Serialize)]
pub struct SelfTestConfig {
    pub namespace: String,
    pub workload_source: WorkloadSource,
    pub cleanup_mode: CleanupMode,
    pub execution_mode: ExecutionMode,
    #[serde(skip)]
    pub timeout: Duration,
    pub sriov_network: Option<String>,
    pub gpu_requirement: GpuRequirement,
    pub signal_handling: SignalHandling,
    pub workload: Option<String>,
    pub image: String,
    pub load_from: Option<String>,
    pub gpus_per_node: Option<u32>,
    // image cache detection config
    pub cache_check: ImageCacheCheck,
    pub cache_ttl_seconds: u64,
    #[serde(skip)]
    pub cache_check_timeout: Duration,
    pub topology_rule: Option<String>,
    pub ucx_gid_index: Option<String>,
}

impl Object for SelfTestConfig {
    fn get_value(self: &std::sync::Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "namespace" => Some(Value::from(self.namespace.clone())),
            "image" => Some(Value::from(self.image.clone())),
            "gpus_per_node" => Some(Value::from(self.gpus_per_node)),
            "gpu_requirement" => Some(Value::from_object(self.gpu_requirement)),
            _ => None,
        }
    }
}

/// Represents a selected node pair for testing
#[derive(Debug, Clone, Serialize)]
pub struct NodePair {
    pub node1: SelectedNode,
    pub node2: SelectedNode,
    pub selection_reason: String,
}

impl Object for NodePair {
    fn get_value(self: &std::sync::Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "node1" | "server_node" => Some(Value::from_object(self.node1.clone())),
            "node2" | "client_node" => Some(Value::from_object(self.node2.clone())),
            "selection_reason" => Some(Value::from(self.selection_reason.clone())),
            _ => None,
        }
    }
}

/// A node selected for testing with its RDMA capabilities
#[derive(Debug, Clone, Serialize)]
pub struct SelectedNode {
    pub name: String,
    pub rdma_interfaces: Vec<RdmaInterface>,
    pub topology_block: Option<String>,
    pub platform_specific_info: HashMap<String, String>,
    pub rdma_resource: Option<String>,
}

impl Object for SelectedNode {
    fn get_value(self: &std::sync::Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "name" => Some(Value::from(self.name.clone())),
            "rdma_device" => {
                // helper to get the first RDMA device name
                Some(Value::from(
                    self.rdma_interfaces
                        .first()
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| "mlx5_0".to_string()),
                ))
            }
            "rdma_interfaces" => Some(Value::from_serialize(&self.rdma_interfaces)),
            "topology_block" => Some(Value::from(self.topology_block.clone())),
            "platform_specific_info" => Some(Value::from_serialize(&self.platform_specific_info)),
            "rdma_resource" => Some(Value::from(self.rdma_resource.clone())),
            _ => None,
        }
    }
}

/// RDMA interface information for test workloads
#[derive(Debug, Clone, Serialize)]
pub struct RdmaInterface {
    pub name: String,
    pub device_type: String, // e.g., "mlx5_0", "roce", "ib"
    pub speed: Option<String>,
    pub state: String,
    pub ip_address: Option<String>,
    pub subnet: Option<String>,
}

/// Test execution state and results
#[derive(Debug, Clone, Serialize)]
pub struct TestExecution {
    pub test_id: String,
    pub node_pair: NodePair,
    pub workload_name: String,
    pub workload_description: String,
    pub status: TestStatus,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub pod_logs: HashMap<String, Vec<String>>, // pod_name -> log_lines
    pub results: TestResults,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum TestStatus {
    Pending,
    Deploying,
    Running,
    Completed,
    Failed,
    TimedOut,
}

/// Aggregated test results
#[derive(Debug, Clone, Serialize, Default)]
pub struct TestResults {
    pub success: bool,
    pub latency_ms: Option<f64>,
    pub bandwidth_gbps: Option<f64>,
    pub error_messages: Vec<String>,
    pub rdma_connection_established: bool,
    pub packet_loss_percent: Option<f64>,
    pub detailed_metrics: HashMap<String, serde_json::Value>,
}

/// Main self-test orchestrator
pub struct SelfTestOrchestrator {
    client: Client,
    config: SelfTestConfig,
    test_execution: Arc<Mutex<Option<TestExecution>>>,
    detected_sriov_network: Arc<Mutex<Option<String>>>,
}

/// Job status for monitoring
#[derive(Debug)]
struct JobStatus {
    completed: bool,
    failed: bool,
}

/// RDMA interface detection functions
impl RdmaInterface {
    /// Extract RDMA interfaces from a NodeInfo based on platform type
    pub fn extract_from_node(node_info: &NodeInfo) -> Vec<RdmaInterface> {
        match node_info.platform_type {
            PlatformType::CoreWeave => Self::extract_coreweave_rdma(node_info),
            PlatformType::GKE => Self::extract_gke_rdma(node_info),
            PlatformType::OpenShift => Self::extract_openshift_rdma(node_info),
            PlatformType::GenericKubernetes => Self::extract_generic_rdma(node_info),
        }
    }

    /// Extract CoreWeave RDMA interface information
    fn extract_coreweave_rdma(node_info: &NodeInfo) -> Vec<RdmaInterface> {
        let mut interfaces = Vec::new();

        if !node_info.rdma_capability.is_capable() {
            return interfaces;
        }

        // CoreWeave uses labels like ib.coreweave.cloud/speed
        let device_type = node_info
            .rdma_type
            .as_ref()
            .map(|t| {
                if t.contains("RoCE") {
                    "roce"
                } else if t.contains("InfiniBand") {
                    "ib"
                } else {
                    "unknown"
                }
            })
            .unwrap_or("unknown");

        // extract speed from IB speed label
        let speed = node_info.ib_speed.clone();

        // look for mellanox NICs for interface names
        for nic in &node_info.mellanox_nics {
            interfaces.push(RdmaInterface {
                name: nic.interface.clone(),
                device_type: device_type.to_string(),
                speed: speed.clone(),
                state: "active".to_string(), // assume active if RDMA capable
                ip_address: None,            // CoreWeave doesn't expose IP in scan
                subnet: None,
            });
        }

        // fallback if no mellanox NICs found but RDMA capable
        if interfaces.is_empty() && node_info.rdma_capability.is_capable() {
            interfaces.push(RdmaInterface {
                name: "mlx5_0".to_string(), // common default
                device_type: device_type.to_string(),
                speed,
                state: "active".to_string(),
                ip_address: None,
                subnet: None,
            });
        }

        interfaces
    }

    /// Extract GKE RDMA interface information
    fn extract_gke_rdma(node_info: &NodeInfo) -> Vec<RdmaInterface> {
        let mut interfaces = Vec::new();

        if !node_info.rdma_capability.is_capable() {
            return interfaces;
        }

        // extract GKE data if available
        let gke_data = match &node_info.platform_data {
            crate::models::PlatformSpecificData::Gke(data) => data,
            _ => return interfaces,
        };

        // GKE has detailed RDMA interface info
        for gke_iface in &gke_data.rdma_interfaces {
            // convert GKE interface birth_name to device name
            let device_name = if gke_iface.birth_name.contains("rdma") {
                // convert gpu0rdma0 to mlx5_0
                let gpu_num = gke_iface
                    .birth_name
                    .chars()
                    .find(|c| c.is_ascii_digit())
                    .unwrap_or('0');
                format!("mlx5_{}", gpu_num)
            } else {
                "mlx5_0".to_string()
            };

            interfaces.push(RdmaInterface {
                name: device_name,
                device_type: "roce".to_string(), // GKE uses RoCE
                speed: Some("100G".to_string()), // typical GKE RDMA speed
                state: "active".to_string(),
                ip_address: Some(gke_iface.ip_address.clone()),
                subnet: Some(gke_iface.subnet.clone()),
            });
        }

        // fallback if no detailed interfaces but RDMA capable
        if interfaces.is_empty() && node_info.rdma_capability.is_capable() {
            interfaces.push(RdmaInterface {
                name: "mlx5_0".to_string(),
                device_type: "roce".to_string(),
                speed: Some("100G".to_string()),
                state: "active".to_string(),
                ip_address: None,
                subnet: None,
            });
        }

        interfaces
    }

    /// Extract OpenShift RDMA interface information
    fn extract_openshift_rdma(node_info: &NodeInfo) -> Vec<RdmaInterface> {
        let mut interfaces = Vec::new();

        if !node_info.rdma_capability.is_capable() {
            return interfaces;
        }

        // OpenShift detection based on feature labels
        let device_type = if node_info
            .node_labels
            .contains_key("feature.node.kubernetes.io/rdma-roce")
        {
            "roce"
        } else if node_info
            .node_labels
            .contains_key("feature.node.kubernetes.io/rdma-ib")
        {
            "ib"
        } else {
            "unknown"
        };

        // look for RDMA devices in labels
        for key in node_info.node_labels.keys() {
            if key.starts_with("feature.node.kubernetes.io/pci-15b3") {
                // Mellanox vendor ID
                // extract device name from label if possible
                let device_name = if key.contains("mlx5") {
                    "mlx5_0".to_string()
                } else if key.contains("mlx4") {
                    "mlx4_0".to_string()
                } else {
                    "mlx5_0".to_string() // default
                };

                interfaces.push(RdmaInterface {
                    name: device_name,
                    device_type: device_type.to_string(),
                    speed: None, // OpenShift doesn't expose speed in labels
                    state: "active".to_string(),
                    ip_address: None,
                    subnet: None,
                });
                break; // usually just one RDMA device per node
            }
        }

        // fallback
        if interfaces.is_empty() && node_info.rdma_capability.is_capable() {
            interfaces.push(RdmaInterface {
                name: "mlx5_0".to_string(),
                device_type: device_type.to_string(),
                speed: None,
                state: "active".to_string(),
                ip_address: None,
                subnet: None,
            });
        }

        interfaces
    }

    /// Extract generic Kubernetes RDMA interface information
    fn extract_generic_rdma(node_info: &NodeInfo) -> Vec<RdmaInterface> {
        let mut interfaces = Vec::new();

        if !node_info.rdma_capability.is_capable() {
            return interfaces;
        }

        // basic fallback for generic Kubernetes
        interfaces.push(RdmaInterface {
            name: "mlx5_0".to_string(),
            device_type: "unknown".to_string(),
            speed: None,
            state: "active".to_string(),
            ip_address: None,
            subnet: None,
        });

        interfaces
    }
}

/// Node selection utilities
impl SelectedNode {
    /// Convert a NodeInfo to a SelectedNode for testing
    pub fn from_node_info(node_info: &NodeInfo) -> Self {
        let rdma_interfaces = RdmaInterface::extract_from_node(node_info);

        let mut platform_specific_info = HashMap::new();

        // add platform-specific metadata
        match node_info.platform_type {
            PlatformType::CoreWeave => {
                if let Some(ref fabric) = node_info.ib_fabric {
                    platform_specific_info.insert("ib_fabric".to_string(), fabric.clone());
                }
                if let Some(ref leafgroup) = node_info.leafgroup {
                    platform_specific_info.insert("leafgroup".to_string(), leafgroup.clone());
                }
                if let Some(ref speed) = node_info.ib_speed {
                    platform_specific_info.insert("ib_speed".to_string(), speed.clone());
                }
            }
            PlatformType::GKE => {
                if let crate::models::PlatformSpecificData::Gke(gke_data) = &node_info.platform_data
                {
                    if let Some(ref nodepool) = gke_data.nodepool {
                        platform_specific_info.insert("nodepool".to_string(), nodepool.clone());
                    }
                    if let Some(ref zone) = gke_data.zone {
                        platform_specific_info.insert("zone".to_string(), zone.clone());
                    }
                    if let Some(ref fabric_domain) = gke_data.fabric_domain {
                        platform_specific_info
                            .insert("fabric_domain".to_string(), fabric_domain.clone());
                    }
                }
            }
            _ => {}
        }

        SelectedNode {
            name: node_info.name.clone(),
            rdma_interfaces,
            topology_block: node_info.topology_block.clone(),
            platform_specific_info,
            rdma_resource: node_info.rdma_resource.clone(),
        }
    }
}

impl SelfTestOrchestrator {
    pub fn new(client: Client, config: SelfTestConfig) -> Self {
        Self {
            client,
            config,
            test_execution: Arc::new(Mutex::new(None)),
            detected_sriov_network: Arc::new(Mutex::new(None)),
        }
    }

    /// detect available SR-IOV networks in the given namespace
    /// looks in the openshift-sriov-network-operator namespace and filters by networkNamespace
    async fn detect_sriov_networks(&self, target_namespace: &str) -> Result<Vec<SriovNetwork>> {
        // openshift SR-IOV networks are defined in the operator namespace
        let operator_namespace = "openshift-sriov-network-operator";
        let sriov_api: Api<SriovNetwork> = Api::namespaced(self.client.clone(), operator_namespace);

        match sriov_api.list(&ListParams::default()).await {
            Ok(network_list) => {
                // filter networks that target our namespace
                let matching_networks: Vec<SriovNetwork> = network_list
                    .items
                    .into_iter()
                    .filter(|net| {
                        // check if networkNamespace matches our target namespace
                        if let Some(ref ns) = net.spec.network_namespace {
                            ns == target_namespace
                        } else {
                            // if not specified, it defaults to the same namespace as the network resource
                            false
                        }
                    })
                    .collect();

                info!(
                    "Found {} SR-IOV networks targeting namespace {}",
                    matching_networks.len(),
                    target_namespace
                );
                Ok(matching_networks)
            }
            Err(e) => {
                // sr-iov might not be available on this cluster
                tracing::debug!(
                    "Failed to list SR-IOV networks (cluster may not support SR-IOV): {}",
                    e
                );
                Ok(Vec::new())
            }
        }
    }

    /// select the best SR-IOV network for RDMA testing based on node pair
    fn select_sriov_network(
        &self,
        networks: &[SriovNetwork],
        _node_pair: &NodePair,
    ) -> Option<String> {
        if networks.is_empty() {
            return None;
        }

        // prefer networks with "rdma" or "roce" in the name
        let rdma_network = networks.iter().find(|net| {
            if let Some(name) = &net.metadata.name {
                let lower = name.to_lowercase();
                lower.contains("rdma") || lower.contains("roce")
            } else {
                false
            }
        });

        if let Some(net) = rdma_network {
            return net.metadata.name.clone();
        }

        // fallback: check resource_name field for RDMA-related resources
        let rdma_resource_network = networks.iter().find(|net| {
            let resource_name = &net.spec.resource_name;
            let lower = resource_name.to_lowercase();
            lower.contains("rdma") || lower.contains("roce") || lower.contains("mlnx")
        });

        if let Some(net) = rdma_resource_network {
            return net.metadata.name.clone();
        }

        // last resort: use first available network
        networks.first().and_then(|net| net.metadata.name.clone())
    }

    /// Setup signal handler for cleanup on CTRL-C
    async fn setup_signal_handler(&self) {
        if !self.config.signal_handling.should_cleanup_on_signal()
            || self.config.execution_mode.is_dry_run()
        {
            return;
        }

        let client = self.client.clone();
        let namespace = self.config.namespace.clone();
        let test_execution = self.test_execution.clone();

        tokio::spawn(async move {
            match signal::ctrl_c().await {
                Ok(()) => {
                    println!("\n\nReceived interrupt signal (CTRL-C)");

                    let test_exec = test_execution.lock().await;
                    if let Some(ref execution) = *test_exec {
                        println!("Cleaning up test resources...");

                        // cleanup in signal handler
                        if let Err(e) =
                            Self::cleanup_resources_static(&client, &namespace, execution).await
                        {
                            eprintln!("Error during cleanup: {}", e);
                        } else {
                            println!("Cleanup completed");
                        }
                    }

                    std::process::exit(130); // standard exit code for SIGINT
                }
                Err(err) => {
                    eprintln!("Error setting up signal handler: {}", err);
                }
            }
        });
    }

    /// static cleanup helper for signal handler
    async fn cleanup_resources_static(
        client: &Client,
        namespace: &str,
        test_execution: &TestExecution,
    ) -> Result<()> {
        use k8s_openapi::api::batch::v1::Job;
        use k8s_openapi::api::core::v1::{ConfigMap, Service};
        use kube::{Api, api::DeleteParams};

        let test_id_short = &test_execution.test_id[..8];

        // delete jobs
        let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
        let job_names = [
            format!("nixl-test-target-{}", test_id_short),
            format!("nixl-test-initiator-{}", test_id_short),
        ];
        for job_name in &job_names {
            if let Err(e) = jobs.delete(job_name, &DeleteParams::default()).await {
                tracing::debug!("Failed to delete job {}: {}", job_name, e);
            }
        }

        // delete service
        let services: Api<Service> = Api::namespaced(client.clone(), namespace);
        if let Err(e) = services
            .delete("nixl-test-target", &DeleteParams::default())
            .await
        {
            tracing::debug!("Failed to delete service nixl-test-target: {}", e);
        }

        // delete configmap
        let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
        let cm_name = format!("nixl-test-script-{}", test_id_short);
        if let Err(e) = configmaps.delete(&cm_name, &DeleteParams::default()).await {
            tracing::debug!("Failed to delete configmap {}: {}", cm_name, e);
        }

        Ok(())
    }

    /// Build RDMA configuration info for workload rendering
    async fn create_rdma_info(&self, test_execution: &TestExecution) -> workloads::RdmaInfo {
        // extract RDMA resource type from node1, fallback to rdma/ib
        tracing::debug!(
            "Node1 rdma_resource: {:?}",
            test_execution.node_pair.node1.rdma_resource
        );
        let rdma_resource_type = test_execution
            .node_pair
            .node1
            .rdma_resource
            .as_ref()
            .and_then(|r| {
                let resource_type = r.split(':').next().unwrap_or("").trim();
                tracing::debug!("Parsed resource type: '{}'", resource_type);
                if resource_type.is_empty() {
                    None
                } else {
                    Some(resource_type)
                }
            })
            .unwrap_or("rdma/ib")
            .to_string();
        tracing::info!("Using RDMA resource type: {}", rdma_resource_type);

        // determine UCX config and SR-IOV network based on RDMA type
        let (ucx_tls, ucx_gid_index, sriov_network) = if rdma_resource_type.contains("roce") {
            // for RoCE on OpenShift, use SR-IOV network attachment
            tracing::info!("Detected RoCE, configuring for OpenShift SR-IOV");

            // use explicitly configured network, or auto-detected network, or fallback
            let detected = self.detected_sriov_network.lock().await;
            let network = self
                .config
                .sriov_network
                .clone()
                .or_else(|| detected.clone())
                .unwrap_or_else(|| "roce-p2".to_string());

            // for RoCE with SR-IOV, use conservative transport list (rc and tcp only)
            // ud/dc can cause issues with some SR-IOV configurations
            let tls = if self.config.gpu_requirement.requires_gpu() {
                "rc,tcp,cuda_copy,cuda_ipc".to_string()
            } else {
                "rc,tcp".to_string()
            };
            // use CLI-specified GID index, or let UCX auto-detect
            let gid_index = self.config.ucx_gid_index.clone().unwrap_or_default();
            (tls, gid_index, Some(network))
        } else {
            // for InfiniBand, let UCX auto-select or specify full list
            tracing::info!("Detected InfiniBand, configuring UCX transports");
            let tls = if self.config.gpu_requirement.requires_gpu() {
                "rc,ud,dc,tcp,cuda_copy,cuda_ipc,gdr_copy".to_string()
            } else {
                "rc,ud,dc,tcp".to_string()
            };
            (tls, "0".to_string(), None)
        };

        workloads::RdmaInfo {
            rdma_resource_type,
            sriov_network,
            ucx_tls,
            ucx_gid_index,
        }
    }

    /// Run the complete self-test workflow
    pub async fn run(&self) -> Result<TestExecution> {
        // setup signal handler first
        self.setup_signal_handler().await;

        // load workload first to know GPU requirements
        let workload = match self.config.workload_source {
            WorkloadSource::Stdin => self.load_workload_from_stdin().await?,
            WorkloadSource::Embedded => self.load_embedded_workload().await?,
        };

        println!("Analyzing cluster for optimal RDMA node pairing...");

        // analyze cluster and select optimal nodes
        let cluster_report = self.analyze_cluster().await?;
        let node_pair = self.select_optimal_node_pair(&cluster_report, workload.as_ref())?;

        println!(
            "Selected nodes: {} <-> {}",
            node_pair.node1.name, node_pair.node2.name
        );
        println!("Reason: {}", node_pair.selection_reason);

        // auto-detect SR-IOV networks if not explicitly provided and RoCE is detected
        let rdma_type = node_pair
            .node1
            .rdma_resource
            .as_ref()
            .and_then(|r| r.split(':').next())
            .unwrap_or("");
        let is_roce = rdma_type.contains("roce");

        if self.config.sriov_network.is_none() {
            if is_roce {
                println!(
                    "Detecting SR-IOV networks in namespace '{}'...",
                    self.config.namespace
                );
                let sriov_networks = self.detect_sriov_networks(&self.config.namespace).await?;

                if !sriov_networks.is_empty() {
                    if let Some(selected_network) =
                        self.select_sriov_network(&sriov_networks, &node_pair)
                    {
                        println!("Auto-selected SR-IOV network: {}", selected_network);
                        let mut detected = self.detected_sriov_network.lock().await;
                        *detected = Some(selected_network);
                    } else {
                        return Err(anyhow::anyhow!(
                            "No suitable SR-IOV network found for namespace '{}'. \
                             Found {} network(s) but none matched RDMA criteria. \
                             Please specify --sriov-network manually or configure an SR-IOV network for this namespace.",
                            self.config.namespace,
                            sriov_networks.len()
                        ));
                    }
                } else {
                    return Err(anyhow::anyhow!(
                        "No SR-IOV networks found for namespace '{}'. \
                         RoCE RDMA requires SR-IOV network configuration. \
                         Please either:\n  \
                         1. Configure an SR-IOV network in the openshift-sriov-network-operator namespace \
                         with networkNamespace: '{}', or\n  \
                         2. Specify an existing network with --sriov-network <network-name>",
                        self.config.namespace,
                        self.config.namespace
                    ));
                }
            } else {
                tracing::info!("InfiniBand detected, SR-IOV network not required");
            }
        }

        println!("Deploying test workload: {}", workload.name());

        // execute the test
        let test_execution = self.execute_test(node_pair, workload).await?;

        // cleanup unless requested not to
        if self.config.cleanup_mode.should_cleanup() && !self.config.execution_mode.is_dry_run() {
            println!("Cleaning up test resources...");
            self.cleanup_test_resources(&test_execution).await?;
        }

        Ok(test_execution)
    }

    async fn analyze_cluster(&self) -> Result<ClusterReport> {
        // reuse existing cluster analysis logic from main.rs
        info!("Starting cluster analysis for self-test...");

        // get cluster URL from kube config
        let kube_config = kube::Config::infer().await?;
        let api_server_url = kube_config.cluster_url.to_string();

        let nodes: Api<Node> = Api::all(self.client.clone());
        let node_list = nodes.list(&ListParams::default()).await?;
        info!("Found {} nodes in cluster", node_list.items.len());

        // detect platform type from first node
        let platform_type = if let Some(first_node) = node_list.items.first() {
            let empty_labels = BTreeMap::new();
            let labels = first_node.metadata.labels.as_ref().unwrap_or(&empty_labels);
            detect_platform_from_labels(labels).get_platform_type()
        } else {
            PlatformType::GenericKubernetes
        };

        // determine cluster-wide topology strategy before analyzing individual nodes
        let cluster_topology_strategy =
            ClusterAnalyzer::determine_cluster_topology_strategy(&node_list.items, &platform_type);

        let mut cluster_report = ClusterReport {
            total_nodes: node_list.items.len(),
            rdma_nodes: 0,
            platform_type,
            api_server_url,
            topology_detection: None,
            rdma_types: Vec::new(),
            topology_blocks: HashMap::new(),
            topology_gpu_counts: HashMap::new(),
            ib_fabrics: Vec::new(),
            superpods: Vec::new(),
            leafgroups: Vec::new(),
            sriov_networks: Vec::new(),
            nodes: Vec::new(),
            gpu_nodes: 0,
            gpu_types: Vec::new(),
            total_gpus: 0,
            image_checked: None,
            cache_check_timestamp: None,
        };

        // determine image to check based on config
        let check_image = if self.config.cache_check.should_check_cache() {
            Some(self.config.image.as_str())
        } else {
            None
        };

        for node in node_list.items {
            let node_info = ClusterAnalyzer::analyze_node_with_image(
                &node,
                LabelDetailLevel::Detailed,
                &cluster_topology_strategy,
                check_image,
                self.config.topology_rule.as_deref(),
            )?;

            // set cluster topology detection from strategy
            if cluster_report.topology_detection.is_none() {
                cluster_report.topology_detection = cluster_topology_strategy.clone();
            }

            if node_info.rdma_capability.is_capable() {
                cluster_report.rdma_nodes += 1;
            }

            // collect GPU statistics
            if let Some(gpu_count) = node_info.gpu_count {
                cluster_report.gpu_nodes += 1;
                cluster_report.total_gpus += gpu_count;
            }

            if let Some(gpu_type) = &node_info.gpu_type
                && !cluster_report.gpu_types.contains(gpu_type)
            {
                cluster_report.gpu_types.push(gpu_type.clone());
            }

            // collect unique RDMA types
            if let Some(rdma_type) = &node_info.rdma_type
                && !cluster_report.rdma_types.contains(rdma_type)
            {
                cluster_report.rdma_types.push(rdma_type.clone());
            }

            // collect topology blocks and GPU counts per block
            if let Some(topology_block) = &node_info.topology_block {
                *cluster_report
                    .topology_blocks
                    .entry(topology_block.clone())
                    .or_insert(0) += 1;

                if let Some(gpu_count) = node_info.gpu_count {
                    // for CoreWeave, aggregate GPUs by fabric instead of leafgroup
                    let aggregation_key = if cluster_report.platform_type == PlatformType::CoreWeave
                    {
                        node_info
                            .ib_fabric
                            .clone()
                            .unwrap_or_else(|| topology_block.clone())
                    } else {
                        topology_block.clone()
                    };

                    *cluster_report
                        .topology_gpu_counts
                        .entry(aggregation_key)
                        .or_insert(0) += gpu_count;
                }
            }

            // collect unique values for summary
            if let Some(fabric) = &node_info.ib_fabric
                && !cluster_report.ib_fabrics.contains(fabric)
            {
                cluster_report.ib_fabrics.push(fabric.clone());
            }

            if let Some(superpod) = &node_info.superpod
                && !cluster_report.superpods.contains(superpod)
            {
                cluster_report.superpods.push(superpod.clone());
            }

            if let Some(leafgroup) = &node_info.leafgroup
                && !cluster_report.leafgroups.contains(leafgroup)
            {
                cluster_report.leafgroups.push(leafgroup.clone());
            }

            // for self-test, we only care about RDMA-capable nodes
            if node_info.rdma_capability.is_capable() {
                cluster_report.nodes.push(node_info);
            }
        }

        // fetch all pods to calculate GPU allocation
        let pods_api: Api<Pod> = Api::all(self.client.clone());
        if let Ok(pod_list) = pods_api.list(&ListParams::default()).await {
            ClusterAnalyzer::populate_gpu_allocations(&mut cluster_report.nodes, &pod_list.items);
        }

        // log image cache results if checked
        if self.config.cache_check.should_check_cache() {
            let cached_count = cluster_report
                .nodes
                .iter()
                .filter(|n| n.image_cache_status.is_cached())
                .count();
            info!(
                "Image cache check complete: {}/{} RDMA nodes have image cached",
                cached_count,
                cluster_report.nodes.len()
            );

            cluster_report.image_checked = Some(self.config.image.clone());
            cluster_report.cache_check_timestamp = cluster_report
                .nodes
                .first()
                .and_then(|n| n.image_cache_checked_at);
        }

        info!(
            "Cluster analysis complete: {} total nodes, {} RDMA-capable",
            cluster_report.total_nodes, cluster_report.rdma_nodes
        );

        Ok(cluster_report)
    }

    /// Check if a node has sufficient free GPUs
    fn has_sufficient_gpus(node: &NodeInfo, required: u32) -> bool {
        // if no GPU info available, assume it doesn't have enough
        let allocatable = match node.gpu_allocatable {
            Some(a) => a,
            None => return false,
        };

        // if no allocation info, conservatively assume all are allocated
        let allocated = node.gpu_allocated.unwrap_or(allocatable);

        let free = allocatable.saturating_sub(allocated);
        free >= required
    }

    fn select_optimal_node_pair(
        &self,
        cluster_report: &ClusterReport,
        workload: &dyn workloads::TestWorkload,
    ) -> Result<NodePair> {
        // get GPU requirement from workload, or use override from config
        let required_gpus = self
            .config
            .gpus_per_node
            .unwrap_or_else(|| workload.required_gpus_per_node());

        // filter to RDMA-capable nodes only
        let mut rdma_nodes: Vec<&NodeInfo> = cluster_report
            .nodes
            .iter()
            .filter(|node| node.rdma_capability.is_capable())
            .collect();

        // if workload requires GPUs, filter nodes with sufficient free GPUs
        if required_gpus > 0 {
            let nodes_before = rdma_nodes.len();
            rdma_nodes.retain(|node| Self::has_sufficient_gpus(node, required_gpus));

            if rdma_nodes.len() < nodes_before {
                info!(
                    "Filtered {} nodes that don't have {} free GPUs",
                    nodes_before - rdma_nodes.len(),
                    required_gpus
                );
            }
        }

        if rdma_nodes.len() < 2 {
            if required_gpus > 0 {
                return Err(anyhow::anyhow!(
                    "Need at least 2 RDMA-capable nodes with {} free GPUs for testing, found: {} RDMA nodes, but only {} with sufficient GPUs",
                    required_gpus,
                    cluster_report.rdma_nodes,
                    rdma_nodes.len()
                ));
            } else {
                return Err(anyhow::anyhow!(
                    "Need at least 2 RDMA-capable nodes for testing, found: {}",
                    rdma_nodes.len()
                ));
            }
        }

        // group nodes by RDMA device type for compatibility testing
        let mut rdma_type_groups: HashMap<String, Vec<&NodeInfo>> = HashMap::new();
        for node in &rdma_nodes {
            let rdma_type = node
                .rdma_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            rdma_type_groups.entry(rdma_type).or_default().push(node);
        }

        // try to find the best pair with same RDMA type, preferably across different topology blocks
        let best_pair = self
            .find_best_node_pair_by_rdma_type(&rdma_type_groups, &cluster_report.platform_type)?;

        if let Some((node1, node2, reason)) = best_pair {
            Ok(NodePair {
                node1: SelectedNode::from_node_info(node1),
                node2: SelectedNode::from_node_info(node2),
                selection_reason: reason,
            })
        } else {
            // fallback: just pick any two RDMA nodes
            let node1 = rdma_nodes[0];
            let node2 = rdma_nodes[1];
            Ok(NodePair {
                node1: SelectedNode::from_node_info(node1),
                node2: SelectedNode::from_node_info(node2),
                selection_reason: "Fallback: Selected first two RDMA-capable nodes".to_string(),
            })
        }
    }

    /// Find the best node pair by RDMA type, preferring same topology block testing
    fn find_best_node_pair_by_rdma_type<'a>(
        &self,
        rdma_type_groups: &HashMap<String, Vec<&'a NodeInfo>>,
        platform_type: &PlatformType,
    ) -> Result<Option<(&'a NodeInfo, &'a NodeInfo, String)>> {
        // find the largest group of nodes with the same RDMA type
        let best_rdma_group = rdma_type_groups
            .iter()
            .filter(|(_, nodes)| nodes.len() >= 2)
            .max_by_key(|(_, nodes)| nodes.len());

        if let Some((rdma_type, nodes)) = best_rdma_group {
            let selector = get_topology_selector(platform_type);
            selector.select_same_topology_pair(rdma_type, nodes)
        } else {
            Ok(None)
        }
    }

    async fn load_workload_from_stdin(&self) -> Result<Box<dyn workloads::TestWorkload>> {
        todo!("Implement stdin workload loading")
    }

    async fn load_embedded_workload(&self) -> Result<Box<dyn workloads::TestWorkload>> {
        // use workload from config, or default to nixl test
        let workload_name = self
            .config
            .workload
            .as_deref()
            .unwrap_or("nixl-transfer-test");
        workloads::get_workload_by_name(workload_name)
            .ok_or_else(|| anyhow::anyhow!("workload '{}' not found in registry", workload_name))
    }

    async fn execute_test(
        &self,
        node_pair: NodePair,
        workload: Box<dyn workloads::TestWorkload>,
    ) -> Result<TestExecution> {
        let test_id = uuid::Uuid::new_v4().to_string();
        let start_time = chrono::Utc::now();

        println!("Preparing test workload: {}", workload.name());
        println!("Description: {}", workload.description());

        // create test execution state
        let mut test_execution = TestExecution {
            test_id: test_id.clone(),
            node_pair,
            workload_name: workload.name().to_string(),
            workload_description: workload.description().to_string(),
            status: TestStatus::Deploying,
            start_time,
            end_time: None,
            pod_logs: HashMap::new(),
            results: TestResults::default(),
        };

        if self.config.execution_mode.is_dry_run() {
            // dry run mode: render manifests to stdout
            self.render_manifests_to_stdout(&test_execution, &*workload)
                .await?;
            test_execution.status = TestStatus::Completed;
            test_execution.end_time = Some(chrono::Utc::now());
            test_execution.results.success = true;
        } else {
            // normal mode: deploy to cluster
            println!("Deploying to cluster...");
            self.deploy_workload(&mut test_execution, &*workload)
                .await?;

            // store test execution for signal handler access
            {
                let mut te = self.test_execution.lock().await;
                *te = Some(test_execution.clone());
            }

            // wait for pods to be ready and monitor execution
            test_execution.status = TestStatus::Running;
            self.monitor_test_execution(&mut test_execution, workload.as_ref())
                .await?;
        }

        Ok(test_execution)
    }

    async fn render_manifests_to_stdout(
        &self,
        test_execution: &TestExecution,
        workload: &dyn workloads::TestWorkload,
    ) -> Result<()> {
        println!(
            "\nRendered test manifests for nodes: {} <-> {}",
            test_execution.node_pair.node1.name, test_execution.node_pair.node2.name
        );
        println!(
            "Selection reason: {}",
            test_execution.node_pair.selection_reason
        );
        println!("\n{}", "=".repeat(80));

        // build RDMA info
        let rdma_info = self.create_rdma_info(test_execution).await;

        // render using the workload's trait method
        let test_id_short = &test_execution.test_id[..8];
        let rendered_manifests = workload.render_manifest(
            test_id_short,
            &test_execution.node_pair,
            &self.config,
            &rdma_info,
        )?;

        // output the rendered manifests
        println!("{}", rendered_manifests);
        println!("{}", "-".repeat(40));

        println!("\nDry run completed - manifests rendered above");

        Ok(())
    }

    async fn deploy_workload(
        &self,
        test_execution: &mut TestExecution,
        workload: &dyn workloads::TestWorkload,
    ) -> Result<()> {
        use k8s_openapi::api::batch::v1::Job;
        use k8s_openapi::api::core::v1::{ConfigMap, Service};
        use kube::{Api, api::PostParams};

        // build RDMA info
        let rdma_info = self.create_rdma_info(test_execution).await;

        // render using the workload's trait method
        let test_id_short = &test_execution.test_id[..8];
        let rendered_manifests = workload.render_manifest(
            test_id_short,
            &test_execution.node_pair,
            &self.config,
            &rdma_info,
        )?;

        // parse the rendered YAML manifest
        let docs: Result<Vec<serde_yaml::Value>, _> =
            serde_yaml::Deserializer::from_str(&rendered_manifests)
                .map(serde_yaml::Value::deserialize)
                .collect();
        let docs = docs?;

        for doc in docs {
            if let Some(kind) = doc.get("kind").and_then(|k| k.as_str()) {
                match kind {
                    "ConfigMap" => {
                        let configmaps: Api<ConfigMap> =
                            Api::namespaced(self.client.clone(), &self.config.namespace);
                        let cm: ConfigMap = serde_yaml::from_value(doc)?;
                        println!(
                            "Creating configmap: {}",
                            cm.metadata.name.as_ref().unwrap_or(&"unknown".to_string())
                        );
                        configmaps.create(&PostParams::default(), &cm).await?;
                    }
                    "Service" => {
                        let services: Api<Service> =
                            Api::namespaced(self.client.clone(), &self.config.namespace);
                        let svc: Service = serde_yaml::from_value(doc)?;
                        println!(
                            "Creating service: {}",
                            svc.metadata.name.as_ref().unwrap_or(&"unknown".to_string())
                        );
                        services.create(&PostParams::default(), &svc).await?;
                    }
                    "Job" => {
                        let jobs: Api<Job> =
                            Api::namespaced(self.client.clone(), &self.config.namespace);
                        let job: Job = serde_yaml::from_value(doc)?;
                        println!(
                            "Creating job: {}",
                            job.metadata.name.as_ref().unwrap_or(&"unknown".to_string())
                        );
                        jobs.create(&PostParams::default(), &job).await?;
                    }
                    _ => {
                        println!("Warning: Skipping unsupported resource kind: {}", kind);
                    }
                }
            }
        }

        Ok(())
    }

    async fn monitor_test_execution(
        &self,
        test_execution: &mut TestExecution,
        workload: &dyn workloads::TestWorkload,
    ) -> Result<()> {
        use std::time::Duration;
        use tokio::time::sleep;

        println!("Monitoring test execution...");

        // spawn log streaming tasks for each pod
        let log_stream_handle = self.spawn_log_streamers(&test_execution.test_id);

        // wait for test completion or timeout
        let timeout = workload.expected_duration();
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            // check job status
            let job_statuses = self.check_job_statuses(&test_execution.test_id).await?;

            if job_statuses.iter().all(|status| status.completed) {
                test_execution.status = TestStatus::Completed;
                test_execution.end_time = Some(chrono::Utc::now());
                test_execution.results.success = true;
                println!("Test completed successfully");
                break;
            }

            if job_statuses.iter().any(|status| status.failed) {
                test_execution.status = TestStatus::Failed;
                test_execution.end_time = Some(chrono::Utc::now());
                println!("Test failed");
                break;
            }

            sleep(Duration::from_secs(5)).await;
        }

        if test_execution.status == TestStatus::Running {
            test_execution.status = TestStatus::TimedOut;
            test_execution.end_time = Some(chrono::Utc::now());
            println!("Test timed out after {:?}", timeout);
        }

        // wait a bit for final logs to flush
        sleep(Duration::from_secs(2)).await;
        drop(log_stream_handle);

        Ok(())
    }

    fn spawn_log_streamers(&self, test_id: &str) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let namespace = self.config.namespace.clone();
        let test_id = test_id.to_string();

        tokio::spawn(async move {
            use futures::TryStreamExt;
            use k8s_openapi::api::core::v1::{Event, Pod};
            use kube::runtime::WatchStreamExt;
            use kube::{Api, api::LogParams};
            use owo_colors::OwoColorize;
            use std::collections::HashMap;
            use std::time::Duration;
            use tokio::sync::mpsc;
            use tokio::time::sleep;

            // create channel for interwoven log output and events
            let (tx, mut rx) = mpsc::unbounded_channel::<(String, String)>();

            let mut pod_colors: HashMap<String, usize> = HashMap::new();
            let mut color_index = 0;

            // spawn task to print logs from channel with colored pod names
            let printer = tokio::spawn(async move {
                while let Some((pod_name, line)) = rx.recv().await {
                    // assign color to pod if not yet assigned
                    let color_idx = *pod_colors.entry(pod_name.clone()).or_insert_with(|| {
                        let idx = color_index % 6;
                        color_index += 1;
                        idx
                    });

                    // colorize pod name based on assigned color
                    let colored_pod = match color_idx {
                        0 => pod_name.cyan().to_string(),
                        1 => pod_name.yellow().to_string(),
                        2 => pod_name.green().to_string(),
                        3 => pod_name.magenta().to_string(),
                        4 => pod_name.blue().to_string(),
                        _ => pod_name.bright_cyan().to_string(),
                    };

                    println!("[{}] {}", colored_pod, line);
                }
            });

            // wait a bit for pods to be created
            sleep(Duration::from_secs(5)).await;

            let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
            let test_id_short = &test_id[..8];

            // discover pods with this test-id
            let lp = kube::api::ListParams::default().labels(&format!("test-id={}", test_id_short));

            let pod_list = match pods.list(&lp).await {
                Ok(list) => list,
                Err(e) => {
                    tracing::warn!("Failed to list pods for log streaming: {}", e);
                    return;
                }
            };

            // collect pod names for event filtering
            let test_pod_names: std::collections::HashSet<String> = pod_list
                .items
                .iter()
                .filter_map(|pod| pod.metadata.name.clone())
                .collect();

            // spawn event watcher for these pods
            let events: Api<Event> = Api::namespaced(client.clone(), &namespace);
            let event_tx = tx.clone();

            let event_watcher = tokio::spawn(async move {
                use futures::StreamExt;
                use kube::runtime::watcher;

                // watch all events in namespace (can't filter by label since events don't inherit pod labels)
                let watcher_config = watcher::Config::default();

                let event_stream = watcher(events, watcher_config).applied_objects();
                futures::pin_mut!(event_stream);

                while let Some(event_result) = event_stream.next().await {
                    match event_result {
                        Ok(event) => {
                            // extract pod name from involved object
                            let pod_name = event
                                .involved_object
                                .name
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string());

                            // only process events for our test pods
                            if !test_pod_names.contains(&pod_name) {
                                continue;
                            }

                            // format event message
                            let reason = event.reason.as_deref().unwrap_or("Unknown");
                            let message = event.message.as_deref().unwrap_or("");
                            let event_type = event.type_.as_deref().unwrap_or("Normal");

                            // only show Warning and Error events, or important Normal events
                            let should_show = event_type != "Normal"
                                || reason == "Pulling"
                                || reason == "Pulled"
                                || reason == "Created"
                                || reason == "Started"
                                || reason == "Failed"
                                || reason == "Scheduled"
                                || reason == "FailedScheduling"
                                || reason == "FailedMount";

                            if should_show {
                                let formatted = if event_type == "Normal" {
                                    format!("EVENT: {} - {}", reason, message)
                                } else {
                                    format!("EVENT [{}]: {} - {}", event_type, reason, message)
                                };

                                let _ = event_tx.send((pod_name, formatted));
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Event watch error: {}", e);
                        }
                    }
                }
            });

            let mut stream_tasks = vec![event_watcher];

            for pod in pod_list.items {
                if let Some(pod_name) = pod.metadata.name {
                    let pods_clone = pods.clone();
                    let pod_name_clone = pod_name.clone();
                    let tx_clone = tx.clone();

                    let task = tokio::spawn(async move {
                        // wait for pod to be ready
                        sleep(Duration::from_secs(3)).await;

                        let log_params = LogParams {
                            follow: true,
                            tail_lines: Some(100),
                            ..Default::default()
                        };

                        match pods_clone.log_stream(&pod_name_clone, &log_params).await {
                            Ok(stream) => {
                                use futures::io::AsyncBufReadExt;

                                let _ = tx_clone.send((
                                    pod_name_clone.clone(),
                                    format!("Logs starting for {}", pod_name_clone),
                                ));

                                let mut lines = stream.lines();
                                while let Ok(Some(line)) = lines.try_next().await {
                                    let _ = tx_clone.send((pod_name_clone.clone(), line));
                                }
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "Could not stream logs from {}: {}",
                                    pod_name_clone,
                                    e
                                );
                            }
                        }
                    });

                    stream_tasks.push(task);
                }
            }

            // wait for all streaming tasks to complete
            for task in stream_tasks {
                let _ = task.await;
            }

            // close channel and wait for printer to finish
            drop(tx);
            let _ = printer.await;
        })
    }

    async fn check_job_statuses(&self, test_id: &str) -> Result<Vec<JobStatus>> {
        use k8s_openapi::api::batch::v1::Job;
        use kube::{Api, api::ListParams};

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.config.namespace);
        let test_id_short = &test_id[..8];

        let lp = ListParams::default().labels(&format!("test-id={}", test_id_short));
        let job_list = jobs.list(&lp).await?;

        let mut statuses = Vec::new();
        for job in job_list.items {
            let completed = job.status.as_ref().and_then(|s| s.succeeded).unwrap_or(0) > 0;
            let failed = job.status.as_ref().and_then(|s| s.failed).unwrap_or(0) > 0;

            statuses.push(JobStatus { completed, failed });
        }

        Ok(statuses)
    }

    async fn cleanup_test_resources(&self, test_execution: &TestExecution) -> Result<()> {
        use k8s_openapi::api::batch::v1::Job;
        use k8s_openapi::api::core::v1::{ConfigMap, Service};
        use kube::{Api, api::DeleteParams};

        let test_id_short = &test_execution.test_id[..8];

        // delete jobs
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.config.namespace);
        let job_names = [
            format!("nixl-test-target-{}", test_id_short),
            format!("nixl-test-initiator-{}", test_id_short),
        ];
        for job_name in &job_names {
            if let Err(e) = jobs.delete(job_name, &DeleteParams::default()).await {
                tracing::warn!("Failed to delete job {}: {}", job_name, e);
            }
        }

        // delete service
        let services: Api<Service> = Api::namespaced(self.client.clone(), &self.config.namespace);
        if let Err(e) = services
            .delete("nixl-test-target", &DeleteParams::default())
            .await
        {
            tracing::warn!("Failed to delete service nixl-test-target: {}", e);
        }

        // delete configmap
        let configmaps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.config.namespace);
        let cm_name = format!("nixl-test-script-{}", test_id_short);
        if let Err(e) = configmaps.delete(&cm_name, &DeleteParams::default()).await {
            tracing::warn!("Failed to delete configmap {}: {}", cm_name, e);
        }

        Ok(())
    }
}

impl Default for SelfTestConfig {
    fn default() -> Self {
        Self {
            namespace: "default".to_string(),
            workload_source: WorkloadSource::Embedded,
            cleanup_mode: CleanupMode::Cleanup,
            execution_mode: ExecutionMode::Execute,
            timeout: Duration::from_secs(300), // 5 minutes
            sriov_network: None,
            gpu_requirement: GpuRequirement::Required,
            signal_handling: SignalHandling::CleanupOnSignal,
            workload: None,
            image: "ghcr.io/llm-d/llm-d-cuda-dev:sha-d58731d@sha256:ba067a81b28546650a5496c3093a21b249c3f0c60d0d305ddcd1907e632e6edd".to_string(),
            load_from: None,
            gpus_per_node: None,
            cache_check: ImageCacheCheck::CheckCache,
            cache_ttl_seconds: 1800, // 30 minutes
            cache_check_timeout: Duration::from_secs(5),
            topology_rule: None,
            ucx_gid_index: None,
        }
    }
}
