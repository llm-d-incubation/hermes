use anyhow::Result;
use chrono::Utc;
use k8s_openapi::api::core::v1::Node;
use std::collections::{BTreeMap, HashMap};

use crate::models::*;
use crate::platforms::*;

/// Shared cluster analysis logic extracted from main.rs and self_test.rs
pub struct ClusterAnalyzer;

impl ClusterAnalyzer {
    /// Analyze a single node and extract all relevant information
    pub fn analyze_node(
        node: &Node,
        detail_level: LabelDetailLevel,
        cluster_topology_strategy: &Option<TopologyDetection>,
        topology_rule: Option<&str>,
    ) -> Result<NodeInfo> {
        Self::analyze_node_with_image(
            node,
            detail_level,
            cluster_topology_strategy,
            None,
            topology_rule,
        )
    }

    /// Analyze a single node with optional image cache detection
    pub fn analyze_node_with_image(
        node: &Node,
        detail_level: LabelDetailLevel,
        cluster_topology_strategy: &Option<TopologyDetection>,
        check_image: Option<&str>,
        topology_rule: Option<&str>,
    ) -> Result<NodeInfo> {
        let name = node.metadata.name.clone().unwrap_or_default();
        let labels = node.metadata.labels.clone().unwrap_or_default();
        let annotations = node.metadata.annotations.clone().unwrap_or_default();

        // detect platform and get platform-specific detector
        let platform_detector = detect_platform_from_labels(&labels);
        let platform_type = platform_detector.get_platform_type();

        // use platform-specific RDMA detection
        let (rdma_cap_bool, rdma_type, rdma_resource) =
            platform_detector.detect_rdma_capability(node);
        let rdma_capability = if rdma_cap_bool {
            RdmaCapability::Capable
        } else {
            RdmaCapability::NotCapable
        };

        // check for GPU capability and type
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let allocatable = node.status.as_ref().and_then(|s| s.allocatable.as_ref());

        let (gpu_count, gpu_type, gpu_allocatable) = if let Some(cap) = capacity {
            if let Some(gpu_quantity) = cap.get("nvidia.com/gpu") {
                let count = gpu_quantity.0.parse::<u32>().unwrap_or(0);
                let gpu_model = labels
                    .get("nvidia.com/gpu.product")
                    .or_else(|| labels.get("gpu.nvidia.com/class"))
                    .or_else(|| labels.get("cloud.google.com/gke-accelerator"))
                    .cloned()
                    .unwrap_or_else(|| "NVIDIA GPU".to_string());

                // get allocatable GPUs (may be less than capacity due to daemon sets)
                let alloc_count = allocatable
                    .and_then(|a| a.get("nvidia.com/gpu"))
                    .and_then(|q| q.0.parse::<u32>().ok())
                    .unwrap_or(count);

                (Some(count), Some(gpu_model), Some(alloc_count))
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        // detect topology block using custom rule, cluster-wide strategy, or platform-specific detection
        let (topology_block, topology_detection, topology_rule_error) = if let Some(rule) =
            topology_rule
        {
            // custom rule supersedes all other detection methods
            use crate::topology_rule::{create_custom_topology_detection, evaluate_topology_rule};
            match evaluate_topology_rule(node, &labels, rule) {
                Ok(Some(result)) => (
                    Some(result),
                    Some(create_custom_topology_detection(rule)),
                    None,
                ),
                Ok(None) => (None, Some(create_custom_topology_detection(rule)), None),
                Err(e) => {
                    // don't log immediately - will be aggregated by caller
                    (None, None, Some(format!("{}", e)))
                }
            }
        } else if cluster_topology_strategy.is_some() {
            let (block, detection) = Self::detect_topology_block_with_strategy(
                node,
                &platform_type,
                &labels,
                &annotations,
                cluster_topology_strategy,
            );
            (block, detection, None)
        } else {
            let (block, detection) =
                platform_detector.detect_topology_block(node, &labels, &annotations);
            (block, detection, None)
        };

        // extract platform-specific information using platform detector
        let platform_info =
            platform_detector.extract_platform_specific_info(node, &labels, &annotations);

        // mellanox NIC detection (only if detailed labels requested)
        let mellanox_nics = if detail_level == LabelDetailLevel::Detailed {
            Self::find_mellanox_nics(&labels)
        } else {
            Vec::new()
        };

        // collect relevant labels based on mode and platform
        let filtered_labels: HashMap<String, String> = if detail_level == LabelDetailLevel::Detailed
        {
            labels
                .iter()
                .filter(|(k, _)| {
                    k.starts_with("ib.coreweave.cloud/")
                        || k.starts_with("net.coreweave.cloud/mellanox")
                        || k.starts_with("backend.coreweave.cloud/")
                        || k.starts_with("feature.node.kubernetes.io/rdma")
                        || k.starts_with("feature.node.kubernetes.io/pci-15b3")
                        || k.starts_with("node.openshift.io/")
                        || k.starts_with("network.nvidia.com/")
                        || k.starts_with("k8s.ovn.org/")
                        || k.starts_with("topology.kubernetes.io/")
                        || k.starts_with("failure-domain.beta.kubernetes.io/")
                        || k.starts_with("cloud.google.com/gke-")
                        || k.starts_with("cloud.google.com/gce-topology-")
                        || k.starts_with("topology.gke.io/")
                        || k.contains("rdma")
                        || k.contains("roce")
                        || k.contains("infiniband")
                        || k.contains("topology")
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else {
            // for basic mode, still include topology-relevant labels for detection
            labels
                .iter()
                .filter(|(k, _)| {
                    k.starts_with("k8s.ovn.org/")
                        || k.starts_with("topology.kubernetes.io/")
                        || k.starts_with("failure-domain.beta.kubernetes.io/")
                        || k.starts_with("ib.coreweave.cloud/leafgroup")
                        || k.starts_with("topology.gke.io/")
                        || k.starts_with("cloud.google.com/gke-nodepool")
                        || k.starts_with("cloud.google.com/gce-topology-")
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };

        let image_cache_status = if let Some(img) = check_image {
            if Self::detect_image_in_node(node, img) {
                ImageCacheStatus::Cached
            } else {
                ImageCacheStatus::NotCached
            }
        } else {
            ImageCacheStatus::Unknown
        };

        // construct platform-specific data based on platform type
        let platform_data = match platform_type {
            PlatformType::GKE => PlatformSpecificData::Gke(Box::new(GkePlatformData {
                nodepool: platform_info.gke_nodepool,
                machine_family: platform_info.gke_machine_family,
                zone: platform_info.gke_zone,
                rdma_interfaces: platform_info.gke_rdma_interfaces,
                pci_topology: platform_info.gke_pci_topology,
                fabric_domain: platform_info.gke_fabric_domain,
                topology_block: platform_info.gke_topology_block,
                topology_subblock: platform_info.gke_topology_subblock,
                topology_host: platform_info.gke_topology_host,
            })),
            _ => PlatformSpecificData::Generic,
        };

        // extract CPU and memory allocatable resources
        let (cpu_allocatable, memory_allocatable) = if let Some(alloc) = allocatable {
            let cpu = alloc.get("cpu").map(|q| q.0.clone());
            let mem = alloc.get("memory").map(|q| q.0.clone());
            (cpu, mem)
        } else {
            (None, None)
        };

        // extract SR-IOV resources from allocatable
        let sriov_resources = Self::extract_sriov_resources(allocatable);

        Ok(NodeInfo {
            name,
            rdma_capability,
            rdma_type,
            rdma_resource,
            platform_type,
            topology_block,
            topology_detection,
            ib_speed: platform_info.ib_speed,
            ib_fabric: platform_info.ib_fabric,
            ib_ports: platform_info.ib_ports,
            leafgroup: platform_info.leafgroup,
            superpod: platform_info.superpod,
            neighbors: platform_info.neighbors,
            mellanox_nics,
            node_labels: filtered_labels,
            gpu_count,
            gpu_type,
            gpu_allocatable,
            gpu_allocated: None, // will be populated later if needed
            cpu_allocatable,
            cpu_allocated: None, // will be populated later if needed
            memory_allocatable,
            memory_allocated: None, // will be populated later if needed
            sriov_resources,
            platform_data,
            image_cache_status,
            image_cache_checked_at: if check_image.is_some() {
                Some(Utc::now())
            } else {
                None
            },
            topology_rule_error,
            roce_config: None, // only populated by scan-roce command
        })
    }

    /// Extract SR-IOV resources from node allocatable resources
    /// Looks for resources containing 'sriov', 'vf', or 'rdma' in their names,
    /// or resources under openshift.io/ namespace (excluding standard k8s resources)
    fn extract_sriov_resources(
        allocatable: Option<
            &std::collections::BTreeMap<
                String,
                k8s_openapi::apimachinery::pkg::api::resource::Quantity,
            >,
        >,
    ) -> HashMap<String, String> {
        let mut sriov_resources = HashMap::new();

        if let Some(alloc) = allocatable {
            for (resource_name, quantity) in alloc {
                let name_lower = resource_name.to_lowercase();

                // detect SR-IOV resources by multiple patterns:
                // 1. Contains 'sriov' or 'vf'
                // 2. Contains 'rdma' (covers openshift.io/p2rdma, etc.)
                // 3. OpenShift SR-IOV resources (openshift.io/* but exclude common openshift resources)
                let is_sriov = name_lower.contains("sriov")
                    || name_lower.contains("vf")
                    || name_lower.contains("rdma")
                    || (name_lower.starts_with("openshift.io/")
                        && !name_lower.contains("hugepages")
                        && !name_lower.contains("cpu")
                        && !name_lower.contains("memory"));

                if is_sriov {
                    sriov_resources.insert(resource_name.clone(), quantity.0.clone());
                }
            }
        }

        sriov_resources
    }

    /// Determine cluster-wide topology strategy before analyzing individual nodes
    pub fn determine_cluster_topology_strategy(
        nodes: &[Node],
        platform_type: &PlatformType,
    ) -> Option<TopologyDetection> {
        match platform_type {
            PlatformType::GKE => {
                // check if any nodes have fabric domains (indicates GPU cluster with RDMA)
                let has_fabric_domains = nodes.iter().any(|node| {
                    if let Some(annotations) = &node.metadata.annotations {
                        // use platform detector to check for fabric domain
                        let empty_labels = BTreeMap::new();
                        let labels = node.metadata.labels.as_ref().unwrap_or(&empty_labels);
                        let detector = detect_platform_from_labels(labels);
                        if detector.get_platform_type() == PlatformType::GKE {
                            let platform_info =
                                detector.extract_platform_specific_info(node, labels, annotations);
                            platform_info.gke_fabric_domain.is_some()
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                });

                if has_fabric_domains {
                    // use hardware topology for the entire cluster
                    Some(TopologyDetection {
                        topology_type: TopologyType::Hardware,
                        detection_method: "GKE RDMA fabric domain analysis".to_string(),
                        confidence: "High".to_string(),
                    })
                } else {
                    // fallback to zone-based topology
                    Some(TopologyDetection {
                        topology_type: TopologyType::Zone,
                        detection_method: "GKE zone+nodepool topology".to_string(),
                        confidence: "Medium".to_string(),
                    })
                }
            }
            _ => None, // let individual node detection handle other platforms
        }
    }

    /// Detect topology block with cluster-wide strategy
    fn detect_topology_block_with_strategy(
        node: &Node,
        platform_type: &PlatformType,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
        cluster_strategy: &Option<TopologyDetection>,
    ) -> (Option<String>, Option<TopologyDetection>) {
        // if we have a cluster-wide strategy, use it
        if let Some(strategy) = cluster_strategy {
            match (&strategy.topology_type, platform_type) {
                (TopologyType::Hardware, PlatformType::GKE) => {
                    // try fabric domain first, fall back to zone+nodepool for non-GPU nodes
                    let detector = detect_platform_from_labels(labels);
                    let platform_info =
                        detector.extract_platform_specific_info(node, labels, annotations);
                    if let Some(fabric_domain) = platform_info.gke_fabric_domain {
                        return (Some(fabric_domain), Some(strategy.clone()));
                    } else {
                        // exclude non-GPU nodes from hardware topology analysis
                        return (None, Some(strategy.clone()));
                    }
                }
                (TopologyType::Zone, PlatformType::GKE) => {
                    // use zone+nodepool for all nodes
                    if let (Some(zone), Some(nodepool)) = (
                        labels
                            .get("topology.gke.io/zone")
                            .or_else(|| labels.get("topology.kubernetes.io/zone")),
                        labels.get("cloud.google.com/gke-nodepool"),
                    ) {
                        return (
                            Some(format!("{}-{}", zone, nodepool)),
                            Some(strategy.clone()),
                        );
                    }
                }
                _ => {
                    // fall back to original detection for other combinations
                }
            }
        }

        // fall back to original per-node detection
        detect_platform_from_labels(labels).detect_topology_block(node, labels, annotations)
    }

    /// Find Mellanox NICs from node labels
    fn find_mellanox_nics(labels: &BTreeMap<String, String>) -> Vec<MellanoxNic> {
        let mut nics = Vec::new();
        let mut interfaces = std::collections::HashSet::new();

        // find all mellanox interfaces
        for key in labels.keys() {
            if let Some(interface) = key.strip_prefix("net.coreweave.cloud/mellanox.")
                && let Some(iface) = interface.split('.').next()
            {
                interfaces.insert(iface.to_string());
            }
        }

        // collect details for each interface
        for interface in interfaces {
            let part_number = labels
                .get(&format!(
                    "net.coreweave.cloud/mellanox.{}.part_number",
                    interface
                ))
                .cloned();
            let firmware = labels
                .get(&format!(
                    "net.coreweave.cloud/mellanox.{}.firmware",
                    interface
                ))
                .cloned();

            nics.push(MellanoxNic {
                part_number,
                firmware,
                interface,
            });
        }

        nics
    }

    /// Detect if a node has a container image cached by checking node.status.images
    ///
    /// This is checked directly from the Node object during scan, not via API calls
    pub fn detect_image_in_node(node: &Node, image: &str) -> bool {
        if let Some(status) = &node.status
            && let Some(images) = &status.images
        {
            for container_image in images {
                // check all names for this image (names is a Vec<String>)
                if let Some(names) = &container_image.names {
                    for name in names {
                        if Self::images_match(name, image) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    fn images_match(image1: &str, image2: &str) -> bool {
        // handle SHA256 digest matching and tag equivalence
        image1 == image2
            || image1.starts_with(image2.split('@').next().unwrap_or(""))
            || image2.starts_with(image1.split('@').next().unwrap_or(""))
    }

    /// Populate GPU allocated counts for each node by querying running pods
    pub fn populate_gpu_allocations(
        nodes: &mut [NodeInfo],
        pods: &[k8s_openapi::api::core::v1::Pod],
    ) {
        use std::collections::HashMap;

        // calculate allocated GPUs per node
        let mut allocated_per_node: HashMap<String, u32> = HashMap::new();

        for pod in pods {
            // skip pods that are not running or scheduled
            let phase = pod.status.as_ref().and_then(|s| s.phase.as_ref());
            if phase != Some(&"Running".to_string()) && phase != Some(&"Pending".to_string()) {
                continue;
            }

            // get node name
            let node_name = match pod.spec.as_ref().and_then(|s| s.node_name.as_ref()) {
                Some(name) => name,
                None => continue,
            };

            // sum GPU requests from all containers
            let gpu_requests: u32 = pod
                .spec
                .as_ref()
                .map(|spec| {
                    spec.containers
                        .iter()
                        .filter_map(|container| {
                            container
                                .resources
                                .as_ref()
                                .and_then(|r| r.requests.as_ref())
                                .and_then(|req| req.get("nvidia.com/gpu"))
                                .and_then(|q| q.0.parse::<u32>().ok())
                        })
                        .sum()
                })
                .unwrap_or(0);

            if gpu_requests > 0 {
                *allocated_per_node.entry(node_name.clone()).or_insert(0) += gpu_requests;
            }
        }

        // update node info with allocated counts
        for node in nodes.iter_mut() {
            node.gpu_allocated = Some(allocated_per_node.get(&node.name).copied().unwrap_or(0));
        }
    }

    /// Populate CPU and memory allocated resources for each node by querying running pods
    pub fn populate_resource_allocations(
        nodes: &mut [NodeInfo],
        pods: &[k8s_openapi::api::core::v1::Pod],
    ) {
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
        use std::collections::HashMap;

        // calculate allocated resources per node
        let mut cpu_allocated_per_node: HashMap<String, Quantity> = HashMap::new();
        let mut memory_allocated_per_node: HashMap<String, Quantity> = HashMap::new();

        for pod in pods {
            // skip pods that are not running or scheduled
            let phase = pod.status.as_ref().and_then(|s| s.phase.as_ref());
            if phase != Some(&"Running".to_string()) && phase != Some(&"Pending".to_string()) {
                continue;
            }

            // get node name
            let node_name = match pod.spec.as_ref().and_then(|s| s.node_name.as_ref()) {
                Some(name) => name,
                None => continue,
            };

            // sum resource requests from all containers
            if let Some(spec) = &pod.spec {
                for container in &spec.containers {
                    if let Some(resources) = &container.resources
                        && let Some(requests) = &resources.requests
                    {
                        // accumulate CPU
                        if let Some(cpu) = requests.get("cpu") {
                            cpu_allocated_per_node
                                .entry(node_name.clone())
                                .and_modify(|total| {
                                    *total = Self::add_quantities(total, cpu);
                                })
                                .or_insert_with(|| cpu.clone());
                        }

                        // accumulate memory
                        if let Some(mem) = requests.get("memory") {
                            memory_allocated_per_node
                                .entry(node_name.clone())
                                .and_modify(|total| {
                                    *total = Self::add_quantities(total, mem);
                                })
                                .or_insert_with(|| mem.clone());
                        }
                    }
                }
            }
        }

        // update node info with allocated counts
        for node in nodes.iter_mut() {
            node.cpu_allocated = cpu_allocated_per_node.get(&node.name).map(|q| q.0.clone());
            node.memory_allocated = memory_allocated_per_node
                .get(&node.name)
                .map(|q| q.0.clone());
        }
    }

    /// helper to add two kubernetes quantities (simplified - just sums the raw strings)
    fn add_quantities(
        q1: &k8s_openapi::apimachinery::pkg::api::resource::Quantity,
        q2: &k8s_openapi::apimachinery::pkg::api::resource::Quantity,
    ) -> k8s_openapi::apimachinery::pkg::api::resource::Quantity {
        // for simplicity, we'll parse millicores for CPU and bytes for memory
        // this is a simplified implementation - a real one would handle all unit types

        // try to parse as millicores (for CPU) or raw number using i128 to avoid overflow
        let parse_value = |s: &str| -> Result<i128, ()> {
            if s.ends_with('m') {
                // millicores
                s.trim_end_matches('m').parse().map_err(|_| ())
            } else if let Ok(val) = s.parse::<f64>() {
                // cores to millicores
                Ok((val * 1000.0) as i128)
            } else if s.ends_with("Ki") {
                // kibibytes
                let v: i128 = s.trim_end_matches("Ki").parse().map_err(|_| ())?;
                v.checked_mul(1024).ok_or(())
            } else if s.ends_with("Mi") {
                // mebibytes
                let v: i128 = s.trim_end_matches("Mi").parse().map_err(|_| ())?;
                v.checked_mul(1024 * 1024).ok_or(())
            } else if s.ends_with("Gi") {
                // gibibytes
                let v: i128 = s.trim_end_matches("Gi").parse().map_err(|_| ())?;
                v.checked_mul(1024 * 1024 * 1024).ok_or(())
            } else {
                s.parse().map_err(|_| ())
            }
        };

        if let (Ok(v1), Ok(v2)) = (parse_value(&q1.0), parse_value(&q2.0)) {
            if let Some(sum) = v1.checked_add(v2) {
                // determine unit based on first quantity and convert back
                if q1.0.ends_with('m') || q2.0.ends_with('m') {
                    k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!("{}m", sum))
                } else if q1.0.ends_with("Gi") || q2.0.ends_with("Gi") {
                    // sum is in bytes, convert back to Gi
                    let gi = sum / (1024 * 1024 * 1024);
                    k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!("{}Gi", gi))
                } else if q1.0.ends_with("Mi") || q2.0.ends_with("Mi") {
                    // sum is in bytes, convert back to Mi
                    let mi = sum / (1024 * 1024);
                    k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!("{}Mi", mi))
                } else if q1.0.ends_with("Ki") || q2.0.ends_with("Ki") {
                    // sum is in bytes, convert back to Ki
                    let ki = sum / 1024;
                    k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!("{}Ki", ki))
                } else {
                    // for CPU cores (no suffix), sum is in millicores, just return the value
                    k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!("{}", sum))
                }
            } else {
                // overflow occurred, just return first quantity
                q1.clone()
            }
        } else {
            // fallback to keeping first quantity if parsing fails
            q1.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_node_gke_with_rdma() {
        // create a mock GKE node with RDMA capabilities
        let node = create_mock_gke_node();
        let result =
            ClusterAnalyzer::analyze_node(&node, LabelDetailLevel::Basic, &None, None).unwrap();

        assert_eq!(result.platform_type, PlatformType::GKE);
        assert_eq!(result.rdma_capability, RdmaCapability::Capable);
        assert_eq!(result.gpu_count, Some(8));
        assert!(result.topology_rule_error.is_none());
    }

    #[test]
    fn test_determine_topology_strategy_gke_with_fabric() {
        let nodes = vec![create_mock_gke_node_with_fabric()];
        let result =
            ClusterAnalyzer::determine_cluster_topology_strategy(&nodes, &PlatformType::GKE);

        assert!(result.is_some());
        let strategy = result.unwrap();
        assert_eq!(strategy.topology_type, TopologyType::Hardware);
    }

    #[test]
    fn test_sriov_resource_detection() {
        // create a mock node with SR-IOV resources
        let node = create_mock_node_with_sriov();
        let result =
            ClusterAnalyzer::analyze_node(&node, LabelDetailLevel::Basic, &None, None).unwrap();

        // verify SR-IOV resources were detected
        assert!(!result.sriov_resources.is_empty());
        assert_eq!(
            result.sriov_resources.get("openshift.io/sriov-vf"),
            Some(&"8".to_string())
        );
        assert_eq!(
            result
                .sriov_resources
                .get("intel.com/intel_sriov_netdevice"),
            Some(&"4".to_string())
        );
    }

    // helper functions to create mock nodes for testing
    fn create_mock_gke_node() -> Node {
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

        let mut labels = BTreeMap::new();
        labels.insert(
            "cloud.google.com/gke-nodepool".to_string(),
            "gpu-pool".to_string(),
        );
        labels.insert(
            "topology.gke.io/zone".to_string(),
            "us-central1-a".to_string(),
        );

        let mut capacity = BTreeMap::new();
        capacity.insert("nvidia.com/gpu".to_string(), Quantity("8".to_string()));
        capacity.insert(
            "networking.gke.io.networks/rdma-0".to_string(),
            Quantity("1".to_string()),
        );

        Node {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-node-1".to_string()),
                labels: Some(labels),
                annotations: Some(BTreeMap::new()),
                ..Default::default()
            },
            status: Some(k8s_openapi::api::core::v1::NodeStatus {
                capacity: Some(capacity),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn create_mock_gke_node_with_fabric() -> Node {
        let mut node = create_mock_gke_node();

        let mut annotations = BTreeMap::new();
        annotations.insert(
            "networking.gke.io/networks".to_string(),
            r#"[{"name":"rdma-0","cidrs":["192.168.1.0/24"]}]"#.to_string(),
        );

        node.metadata.annotations = Some(annotations);
        node
    }

    fn create_mock_node_with_sriov() -> Node {
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

        let labels = BTreeMap::new();

        let mut allocatable = BTreeMap::new();
        allocatable.insert("cpu".to_string(), Quantity("16".to_string()));
        allocatable.insert("memory".to_string(), Quantity("64Gi".to_string()));
        // add SR-IOV resources
        allocatable.insert(
            "openshift.io/sriov-vf".to_string(),
            Quantity("8".to_string()),
        );
        allocatable.insert(
            "intel.com/intel_sriov_netdevice".to_string(),
            Quantity("4".to_string()),
        );

        Node {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-sriov-node".to_string()),
                labels: Some(labels),
                annotations: Some(BTreeMap::new()),
                ..Default::default()
            },
            status: Some(k8s_openapi::api::core::v1::NodeStatus {
                allocatable: Some(allocatable),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
