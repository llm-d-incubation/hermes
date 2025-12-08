use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::models::{NodeInfo, PlatformType};
use crate::self_test::SelfTestConfig;

/// resource requests and limits for test pods
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    /// rdma resource type (e.g., rdma/ib, rdma/roce_gdr)
    pub rdma: String,
    /// gpu resource type (e.g., nvidia.com/gpu)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// resource requests
    pub requests: ResourceQuantities,
    /// resource limits
    pub limits: ResourceQuantities,
}

/// cpu and memory resource quantities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuantities {
    pub memory: String,
    pub cpu: String,
}

/// ucx configuration for rdma transport
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UcxConfig {
    /// ucx log level (info, debug, trace)
    pub log_level: String,
    /// comma-separated list of ucx transports
    pub transports: String,
    /// gid index for roce (string to support empty/auto-detect)
    pub gid_index: String,
}

/// node information for helm values
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmNode {
    /// node name
    pub name: String,
    /// number of gpus on this node
    pub gpus: u32,
    /// rank in distributed setup (0-indexed)
    pub rank: u32,
    /// topology block identifier (leafgroup, fabric domain, zone, etc)
    #[serde(rename = "topologyBlock")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology_block: Option<String>,
}

/// topology summary across all nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologySummary {
    /// total number of nodes
    pub total_nodes: u32,
    /// total number of gpus across all nodes
    pub total_gpus: u32,
    /// gpus per node (assumes homogeneous)
    pub gpus_per_node: u32,
    /// world size for distributed training (total gpus)
    pub world_size: u32,
}

/// topology configuration for multi-node setup
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Topology {
    /// list of nodes in the topology
    pub nodes: Vec<HelmNode>,
    /// summary statistics
    pub summary: TopologySummary,
    /// rdma resource type used (e.g., rdma/ib)
    pub rdma_type: String,
    /// detected platform (CoreWeave, GKE, OpenShift, etc)
    pub platform: String,
    /// whether all nodes are in the same topology block
    pub all_same_block: bool,
}

/// complete helm values for rdma test deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestValues {
    /// unique test identifier (truncated uuid)
    pub test_id: String,
    /// kubernetes namespace for deployment
    pub namespace: String,
    /// job timeout in seconds
    pub active_deadline_seconds: u64,
    /// container image for test pods
    pub image: String,
    /// resource configuration
    pub resources: Resources,
    /// ucx transport configuration
    pub ucx: UcxConfig,
    /// topology and node configuration
    pub topology: Topology,
    /// sr-iov network name for roce (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sriov_network: Option<String>,
}

impl TestValues {
    /// create helm values from a pair of nodes and test configuration
    pub fn from_node_pair(
        node1: &NodeInfo,
        node2: &NodeInfo,
        config: &SelfTestConfig,
        test_id: &str,
    ) -> Result<Self> {
        // validate that both nodes are rdma-capable
        if !node1.rdma_capability.is_capable() || !node2.rdma_capability.is_capable() {
            anyhow::bail!("Both nodes must be RDMA-capable for testing");
        }

        // extract rdma resource type from node1
        let rdma_resource = extract_rdma_resource(node1)?;

        // use provided test id (already truncated to 8 chars by caller)
        let test_id = test_id.to_string();

        // determine if this is roce or infiniband
        let is_roce = rdma_resource.contains("roce");

        // configure ucx transports based on rdma type and gpu requirement
        let (ucx_transports, ucx_gid_index) = if is_roce {
            // roce with sriov: conservative transport list
            let transports = if config.gpu_requirement.requires_gpu() {
                "rc,tcp,cuda_copy,cuda_ipc".to_string()
            } else {
                "rc,tcp".to_string()
            };
            let gid_index = config.ucx_gid_index.clone().unwrap_or_default();
            (transports, gid_index)
        } else {
            // infiniband: full transport list
            let transports = if config.gpu_requirement.requires_gpu() {
                "rc,ud,dc,tcp,cuda_copy,cuda_ipc,gdr_copy".to_string()
            } else {
                "rc,ud,dc,tcp".to_string()
            };
            (transports, "0".to_string())
        };

        // get gpu count per node
        let gpus_per_node = config.gpus_per_node.unwrap_or(0);

        // build topology nodes
        let helm_nodes = vec![
            HelmNode {
                name: node1.name.clone(),
                gpus: gpus_per_node,
                rank: 0,
                topology_block: node1.topology_block.clone(),
            },
            HelmNode {
                name: node2.name.clone(),
                gpus: gpus_per_node,
                rank: 1,
                topology_block: node2.topology_block.clone(),
            },
        ];

        // calculate topology summary
        let total_gpus = gpus_per_node * 2;
        let summary = TopologySummary {
            total_nodes: 2,
            total_gpus,
            gpus_per_node,
            world_size: total_gpus,
        };

        // check if both nodes are in the same topology block
        let all_same_block = match (&node1.topology_block, &node2.topology_block) {
            (Some(b1), Some(b2)) => b1 == b2,
            _ => false,
        };

        // build topology
        let topology = Topology {
            nodes: helm_nodes,
            summary,
            rdma_type: rdma_resource.clone(),
            platform: platform_to_string(node1.platform_type),
            all_same_block,
        };

        // build resources
        let gpu_resource = if config.gpu_requirement.requires_gpu() {
            Some("nvidia.com/gpu".to_string())
        } else {
            None
        };

        let resources = Resources {
            rdma: rdma_resource,
            gpu: gpu_resource,
            requests: ResourceQuantities {
                memory: "2Gi".to_string(),
                cpu: "1".to_string(),
            },
            limits: ResourceQuantities {
                memory: "4Gi".to_string(),
                cpu: "2".to_string(),
            },
        };

        // build ucx config
        let ucx = UcxConfig {
            log_level: "info".to_string(),
            transports: ucx_transports,
            gid_index: ucx_gid_index,
        };

        Ok(TestValues {
            test_id,
            namespace: config.namespace.clone(),
            active_deadline_seconds: 180, // default 3 minutes
            image: config.image.clone(),
            resources,
            ucx,
            topology,
            sriov_network: config.sriov_network.clone(),
        })
    }

    /// serialize to yaml string
    pub fn to_yaml_string(&self) -> Result<String> {
        serde_yaml::to_string(self).context("Failed to serialize TestValues to YAML")
    }

    /// write values to a file
    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let yaml = self.to_yaml_string()?;
        std::fs::write(path, yaml)
            .with_context(|| format!("Failed to write values file to {}", path.display()))
    }
}

/// extract rdma resource type from node info
/// format: "rdma/ib: 1" -> "rdma/ib"
fn extract_rdma_resource(node: &NodeInfo) -> Result<String> {
    node.rdma_resource
        .as_ref()
        .and_then(|r| {
            let resource_type = r.split(':').next().unwrap_or("").trim();
            if resource_type.is_empty() {
                None
            } else {
                Some(resource_type.to_string())
            }
        })
        .ok_or_else(|| anyhow::anyhow!("Node {} has no RDMA resource defined", node.name))
}

/// convert platform type to string for helm values
fn platform_to_string(platform: PlatformType) -> String {
    match platform {
        PlatformType::OpenShift => "OpenShift".to_string(),
        PlatformType::CoreWeave => "CoreWeave".to_string(),
        PlatformType::GKE => "GKE".to_string(),
        PlatformType::GenericKubernetes => "Kubernetes".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PlatformSpecificData, RdmaCapability};
    use std::collections::HashMap;

    fn create_test_node(
        name: &str,
        rdma_resource: &str,
        topology_block: Option<String>,
    ) -> NodeInfo {
        NodeInfo {
            name: name.to_string(),
            rdma_capability: RdmaCapability::Capable,
            rdma_type: Some("InfiniBand".to_string()),
            rdma_resource: Some(rdma_resource.to_string()),
            platform_type: PlatformType::CoreWeave,
            topology_block,
            topology_detection: None,
            ib_speed: Some("200G".to_string()),
            ib_fabric: Some("fabric1".to_string()),
            ib_ports: None,
            leafgroup: Some("371".to_string()),
            superpod: None,
            neighbors: vec![],
            mellanox_nics: vec![],
            node_labels: HashMap::new(),
            gpu_count: Some(8),
            gpu_type: Some("H100-80GB-HBM3".to_string()),
            gpu_allocatable: Some(8),
            gpu_allocated: Some(0),
            cpu_allocatable: None,
            cpu_allocated: None,
            memory_allocatable: None,
            memory_allocated: None,
            sriov_resources: HashMap::new(),
            platform_data: PlatformSpecificData::Generic,
            image_cache_status: crate::models::ImageCacheStatus::Unknown,
            image_cache_checked_at: None,
            topology_rule_error: None,
            roce_config: None,
        }
    }

    #[test]
    fn test_from_node_pair_basic() {
        let node1 = create_test_node("node1", "rdma/ib: 1", Some("371".to_string()));
        let node2 = create_test_node("node2", "rdma/ib: 1", Some("371".to_string()));

        let config = SelfTestConfig {
            namespace: "test-ns".to_string(),
            image: "test-image:latest".to_string(),
            gpus_per_node: Some(8),
            ..Default::default()
        };

        let values = TestValues::from_node_pair(&node1, &node2, &config, "test1234").unwrap();

        assert_eq!(values.test_id, "test1234");
        assert_eq!(values.namespace, "test-ns");
        assert_eq!(values.image, "test-image:latest");
        assert_eq!(values.resources.rdma, "rdma/ib");
        assert_eq!(values.topology.nodes.len(), 2);
        assert_eq!(values.topology.summary.total_nodes, 2);
        assert_eq!(values.topology.summary.total_gpus, 16);
        assert_eq!(values.topology.summary.gpus_per_node, 8);
        assert_eq!(values.topology.platform, "CoreWeave");
        assert!(values.topology.all_same_block);
    }

    #[test]
    fn test_roce_configuration() {
        let node1 = create_test_node("node1", "rdma/roce_gdr: 1", Some("zone1".to_string()));
        let node2 = create_test_node("node2", "rdma/roce_gdr: 1", Some("zone1".to_string()));

        let config = SelfTestConfig {
            gpus_per_node: Some(4),
            ..Default::default()
        };

        let values = TestValues::from_node_pair(&node1, &node2, &config, "roce5678").unwrap();

        assert_eq!(values.test_id, "roce5678");
        assert_eq!(values.resources.rdma, "rdma/roce_gdr");
        assert!(values.ucx.transports.contains("rc"));
        assert!(values.ucx.transports.contains("cuda_copy"));
    }

    #[test]
    fn test_different_topology_blocks() {
        let node1 = create_test_node("node1", "rdma/ib: 1", Some("block1".to_string()));
        let node2 = create_test_node("node2", "rdma/ib: 1", Some("block2".to_string()));

        let config = SelfTestConfig::default();

        let values = TestValues::from_node_pair(&node1, &node2, &config, "diff9abc").unwrap();

        assert_eq!(values.test_id, "diff9abc");
        assert!(!values.topology.all_same_block);
    }

    #[test]
    fn test_yaml_serialization() {
        let node1 = create_test_node("node1", "rdma/ib: 1", Some("371".to_string()));
        let node2 = create_test_node("node2", "rdma/ib: 1", Some("371".to_string()));

        let config = SelfTestConfig {
            gpus_per_node: Some(8),
            ..Default::default()
        };

        let values = TestValues::from_node_pair(&node1, &node2, &config, "yaml1234").unwrap();
        let yaml = values.to_yaml_string().unwrap();

        // basic sanity checks on yaml output
        assert!(yaml.contains("testId: yaml1234"));
        assert!(yaml.contains("namespace:"));
        assert!(yaml.contains("topology:"));
        assert!(yaml.contains("nodes:"));
        assert!(yaml.contains("- name: node1"));
        assert!(yaml.contains("- name: node2"));
    }
}
