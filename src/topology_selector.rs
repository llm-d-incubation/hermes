use crate::models::NodeInfo;
use anyhow::Result;
use std::collections::HashMap;

/// Trait for platform-specific topology-based node selection
pub trait TopologySelector {
    /// Get the topology key for a node (e.g., leafgroup, zone+nodepool, etc.)
    fn get_topology_key(&self, node: &NodeInfo) -> Option<String>;

    /// Format the selection reason for a node pair
    fn format_selection_reason(&self, rdma_type: &str, topology: &str, is_fallback: bool)
    -> String;

    /// Select the best node pair from a group of nodes with the same RDMA type
    fn select_same_topology_pair<'a>(
        &self,
        rdma_type: &str,
        nodes: &[&'a NodeInfo],
    ) -> Result<Option<(&'a NodeInfo, &'a NodeInfo, String)>> {
        // group by topology key
        let topology_groups: HashMap<String, Vec<&'a NodeInfo>> =
            nodes.iter().fold(HashMap::new(), |mut acc, node| {
                if let Some(topology_key) = self.get_topology_key(node) {
                    acc.entry(topology_key).or_default().push(*node);
                }
                acc
            });

        // prefer nodes from the same topology block with multiple nodes
        let best_topology = topology_groups
            .iter()
            .filter(|(_, nodes)| nodes.len() >= 2)
            .max_by_key(|(_, nodes)| nodes.len());

        if let Some((topology_key, topology_nodes)) = best_topology {
            let reason = self.format_selection_reason(rdma_type, topology_key, false);
            return Ok(Some((topology_nodes[0], topology_nodes[1], reason)));
        }

        // fallback: any two nodes with same RDMA type
        if nodes.len() >= 2 {
            let reason = self.format_selection_reason(rdma_type, "unknown", true);
            return Ok(Some((nodes[0], nodes[1], reason)));
        }

        Ok(None)
    }
}

/// CoreWeave-specific topology selector
pub struct CoreWeaveTopologySelector;

impl TopologySelector for CoreWeaveTopologySelector {
    fn get_topology_key(&self, node: &NodeInfo) -> Option<String> {
        node.leafgroup.clone()
    }

    fn format_selection_reason(
        &self,
        rdma_type: &str,
        topology: &str,
        is_fallback: bool,
    ) -> String {
        if is_fallback {
            format!(
                "CoreWeave fallback: {} RDMA nodes (topology may differ)",
                rdma_type
            )
        } else {
            format!(
                "Optimal CoreWeave same-topology: {} RDMA within leafgroup '{}'",
                rdma_type, topology
            )
        }
    }
}

/// GKE-specific topology selector
pub struct GkeTopologySelector;

impl TopologySelector for GkeTopologySelector {
    fn get_topology_key(&self, node: &NodeInfo) -> Option<String> {
        node.topology_block.clone()
    }

    fn format_selection_reason(
        &self,
        rdma_type: &str,
        topology: &str,
        is_fallback: bool,
    ) -> String {
        if is_fallback {
            format!(
                "GKE fallback: {} RDMA nodes (topology may differ)",
                rdma_type
            )
        } else {
            format!(
                "Optimal GKE same-topology: {} RDMA within '{}'",
                rdma_type, topology
            )
        }
    }
}

/// OpenShift-specific topology selector
pub struct OpenShiftTopologySelector;

impl TopologySelector for OpenShiftTopologySelector {
    fn get_topology_key(&self, node: &NodeInfo) -> Option<String> {
        node.topology_block.clone()
    }

    fn format_selection_reason(
        &self,
        rdma_type: &str,
        topology: &str,
        is_fallback: bool,
    ) -> String {
        if is_fallback {
            format!(
                "OpenShift fallback: {} RDMA nodes (topology may differ)",
                rdma_type
            )
        } else {
            format!(
                "Optimal OpenShift same-topology: {} RDMA within '{}'",
                rdma_type, topology
            )
        }
    }
}

/// Generic Kubernetes topology selector
pub struct GenericTopologySelector;

impl TopologySelector for GenericTopologySelector {
    fn get_topology_key(&self, node: &NodeInfo) -> Option<String> {
        node.topology_block.clone()
    }

    fn format_selection_reason(
        &self,
        rdma_type: &str,
        topology: &str,
        is_fallback: bool,
    ) -> String {
        if is_fallback {
            format!(
                "Generic fallback: {} RDMA nodes (topology may differ)",
                rdma_type
            )
        } else {
            format!(
                "Optimal same-topology: {} RDMA within '{}'",
                rdma_type, topology
            )
        }
    }
}

/// Get the appropriate topology selector for a platform type
pub fn get_topology_selector(
    platform_type: &crate::models::PlatformType,
) -> Box<dyn TopologySelector> {
    use crate::models::PlatformType;

    match platform_type {
        PlatformType::CoreWeave => Box::new(CoreWeaveTopologySelector),
        PlatformType::GKE => Box::new(GkeTopologySelector),
        PlatformType::OpenShift => Box::new(OpenShiftTopologySelector),
        PlatformType::GenericKubernetes => Box::new(GenericTopologySelector),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PlatformType, TopologyDetection, TopologyType};
    use insta::assert_snapshot;

    fn create_mock_node(
        name: &str,
        leafgroup: Option<&str>,
        topology_block: Option<&str>,
    ) -> NodeInfo {
        NodeInfo {
            name: name.to_string(),
            rdma_capable: true,
            rdma_type: Some("RoCE".to_string()),
            rdma_resource: Some("rdma/ib: 1".to_string()),
            platform_type: PlatformType::CoreWeave,
            topology_block: topology_block.map(|s| s.to_string()),
            topology_detection: Some(TopologyDetection {
                topology_type: TopologyType::LeafGroup,
                detection_method: "Test".to_string(),
                confidence: "High".to_string(),
            }),
            ib_speed: Some("100G".to_string()),
            ib_fabric: Some("fabric1".to_string()),
            ib_ports: None,
            leafgroup: leafgroup.map(|s| s.to_string()),
            superpod: None,
            neighbors: vec![],
            mellanox_nics: vec![],
            node_labels: HashMap::new(),
            gpu_count: Some(8),
            gpu_type: Some("A100".to_string()),
            gpu_allocatable: Some(8),
            gpu_allocated: Some(0),
            gke_nodepool: None,
            gke_machine_family: None,
            gke_zone: None,
            gke_rdma_interfaces: vec![],
            gke_pci_topology: None,
            gke_fabric_domain: None,
            gke_topology_block: None,
            gke_topology_subblock: None,
            gke_topology_host: None,
        }
    }

    #[test]
    fn test_coreweave_selector_same_leafgroup() {
        let selector = CoreWeaveTopologySelector;
        let nodes = vec![
            create_mock_node("node1", Some("lg1"), Some("lg1")),
            create_mock_node("node2", Some("lg1"), Some("lg1")),
            create_mock_node("node3", Some("lg2"), Some("lg2")),
        ];
        let node_refs: Vec<&NodeInfo> = nodes.iter().collect();

        let result = selector
            .select_same_topology_pair("RoCE", &node_refs)
            .unwrap();
        assert!(result.is_some());

        let (node1, node2, reason) = result.unwrap();
        assert_eq!(node1.name, "node1");
        assert_eq!(node2.name, "node2");
        assert_snapshot!(reason);
    }

    #[test]
    fn test_gke_selector_different_topologies() {
        let selector = GkeTopologySelector;
        let nodes = vec![
            create_mock_node("node1", None, Some("zone-a")),
            create_mock_node("node2", None, Some("zone-b")),
        ];
        let node_refs: Vec<&NodeInfo> = nodes.iter().collect();

        let result = selector
            .select_same_topology_pair("GKE RDMA", &node_refs)
            .unwrap();
        assert!(result.is_some());

        let (_, _, reason) = result.unwrap();
        assert_snapshot!(reason);
    }
}
