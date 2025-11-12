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

    /// Calculate image cache score for a node pair
    /// Used as secondary scoring after topology matching
    fn calculate_cache_score(&self, node1: &NodeInfo, node2: &NodeInfo) -> u8 {
        use crate::models::ImageCacheStatus;

        match (&node1.image_cache_status, &node2.image_cache_status) {
            (ImageCacheStatus::Cached, ImageCacheStatus::Cached) => 3, // both cached - best case
            (ImageCacheStatus::Cached, _) | (_, ImageCacheStatus::Cached) => 2, // one cached
            _ => 1, // neither cached or unknown - still valid pair
        }
    }

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

        // find best pair across all topology groups
        let mut best_pair: Option<(&'a NodeInfo, &'a NodeInfo, String, u8)> = None;

        for (topology_key, topology_nodes) in topology_groups.iter() {
            if topology_nodes.len() >= 2 {
                // find best pair within this topology group by cache score
                for i in 0..topology_nodes.len() {
                    for j in (i + 1)..topology_nodes.len() {
                        let cache_score =
                            self.calculate_cache_score(topology_nodes[i], topology_nodes[j]);
                        let reason = self.format_selection_reason(rdma_type, topology_key, false);

                        if let Some((_, _, _, current_score)) = best_pair {
                            if cache_score > current_score {
                                best_pair = Some((
                                    topology_nodes[i],
                                    topology_nodes[j],
                                    reason,
                                    cache_score,
                                ));
                            }
                        } else {
                            best_pair =
                                Some((topology_nodes[i], topology_nodes[j], reason, cache_score));
                        }
                    }
                }
            }
        }

        if let Some((node1, node2, mut reason, cache_score)) = best_pair {
            // append cache score to reason if cache data is available
            use crate::models::ImageCacheStatus;
            if !matches!(node1.image_cache_status, ImageCacheStatus::Unknown)
                || !matches!(node2.image_cache_status, ImageCacheStatus::Unknown)
            {
                reason = format!("{} (cache score: {})", reason, cache_score);
            }
            return Ok(Some((node1, node2, reason)));
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

    fn create_mock_node(
        name: &str,
        leafgroup: Option<&str>,
        topology_block: Option<&str>,
    ) -> NodeInfo {
        use crate::models::{ImageCacheStatus, RdmaCapability};

        NodeInfo {
            name: name.to_string(),
            rdma_capability: RdmaCapability::Capable,
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
            cpu_allocatable: Some("32".to_string()),
            cpu_allocated: Some("0".to_string()),
            memory_allocatable: Some("128Gi".to_string()),
            memory_allocated: Some("0Gi".to_string()),
            platform_data: crate::models::PlatformSpecificData::Generic,
            image_cache_status: ImageCacheStatus::Unknown,
            image_cache_checked_at: None,
            topology_rule_error: None,
        }
    }

    #[test]
    fn test_coreweave_selector_same_leafgroup() {
        let selector = CoreWeaveTopologySelector;
        let nodes = [
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
        assert!(reason.contains("same-topology") && reason.contains("leafgroup"));
    }

    #[test]
    fn test_gke_selector_different_topologies() {
        let selector = GkeTopologySelector;
        let nodes = [
            create_mock_node("node1", None, Some("zone-a")),
            create_mock_node("node2", None, Some("zone-b")),
        ];
        let node_refs: Vec<&NodeInfo> = nodes.iter().collect();

        let result = selector
            .select_same_topology_pair("GKE RDMA", &node_refs)
            .unwrap();
        assert!(result.is_some());

        let (_, _, reason) = result.unwrap();
        assert!(reason.contains("same-topology") || reason.contains("fallback"));
    }
}
