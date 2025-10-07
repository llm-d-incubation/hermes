use anyhow::Result;
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
        include_detailed_labels: bool,
        cluster_topology_strategy: &Option<TopologyDetection>,
    ) -> Result<NodeInfo> {
        let name = node.metadata.name.clone().unwrap_or_default();
        let labels = node.metadata.labels.clone().unwrap_or_default();
        let annotations = node.metadata.annotations.clone().unwrap_or_default();

        // detect platform and get platform-specific detector
        let platform_detector = detect_platform_from_labels(&labels);
        let platform_type = platform_detector.get_platform_type();

        // use platform-specific RDMA detection
        let (rdma_capable, rdma_type, rdma_resource) =
            platform_detector.detect_rdma_capability(node);

        // check for GPU capability and type
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let (gpu_count, gpu_type) = if let Some(cap) = capacity {
            if let Some(gpu_quantity) = cap.get("nvidia.com/gpu") {
                let count = gpu_quantity.0.parse::<u32>().unwrap_or(0);
                let gpu_model = labels
                    .get("nvidia.com/gpu.product")
                    .or_else(|| labels.get("gpu.nvidia.com/class"))
                    .or_else(|| labels.get("cloud.google.com/gke-accelerator"))
                    .cloned()
                    .unwrap_or_else(|| "NVIDIA GPU".to_string());
                (Some(count), Some(gpu_model))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // detect topology block using cluster-wide strategy or platform-specific detection
        let (topology_block, topology_detection) = if cluster_topology_strategy.is_some() {
            Self::detect_topology_block_with_strategy(
                node,
                &platform_type,
                &labels,
                &annotations,
                cluster_topology_strategy,
            )
        } else {
            platform_detector.detect_topology_block(node, &labels, &annotations)
        };

        // extract platform-specific information using platform detector
        let platform_info =
            platform_detector.extract_platform_specific_info(node, &labels, &annotations);

        // mellanox NIC detection (only if detailed labels requested)
        let mellanox_nics = if include_detailed_labels {
            Self::find_mellanox_nics(&labels)
        } else {
            Vec::new()
        };

        // collect relevant labels based on mode and platform
        let filtered_labels: HashMap<String, String> = if include_detailed_labels {
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

        Ok(NodeInfo {
            name,
            rdma_capable,
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
            gke_nodepool: platform_info.gke_nodepool,
            gke_machine_family: platform_info.gke_machine_family,
            gke_zone: platform_info.gke_zone,
            gke_rdma_interfaces: platform_info.gke_rdma_interfaces,
            gke_pci_topology: platform_info.gke_pci_topology,
            gke_fabric_domain: platform_info.gke_fabric_domain,
            gke_topology_block: platform_info.gke_topology_block,
            gke_topology_subblock: platform_info.gke_topology_subblock,
            gke_topology_host: platform_info.gke_topology_host,
        })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_yaml_snapshot;

    #[test]
    fn test_analyze_node_gke_with_rdma() {
        // create a mock GKE node with RDMA capabilities
        let node = create_mock_gke_node();
        let result = ClusterAnalyzer::analyze_node(&node, false, &None).unwrap();

        assert_yaml_snapshot!(result, {
            ".node_labels" => insta::sorted_redaction(),
        });
    }

    #[test]
    fn test_determine_topology_strategy_gke_with_fabric() {
        let nodes = vec![create_mock_gke_node_with_fabric()];
        let result =
            ClusterAnalyzer::determine_cluster_topology_strategy(&nodes, &PlatformType::GKE);

        assert_yaml_snapshot!(result);
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
}
