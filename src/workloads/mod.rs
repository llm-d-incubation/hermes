use anyhow::Result;
use serde::Serialize;
use std::time::Duration;

use crate::self_test::{NodePair, SelfTestConfig};

pub mod deepep_internode;
pub mod deepep_low_latency;
pub mod deepgemm_minimal;
pub mod deepgemm_simple;
pub mod nixl_transfer;
pub mod pplx_kernels;

/// RDMA configuration info passed to workloads
#[derive(Debug, Clone)]
pub struct RdmaInfo {
    pub rdma_resource_type: String,
    pub sriov_network: Option<String>,
    pub ucx_tls: String,
    pub ucx_gid_index: String,
}

/// Base template node info that most workloads need
#[derive(Debug, Clone, Serialize)]
pub struct TemplateNode {
    pub name: String,
    pub rdma_device: String,
}

/// Trait that all test workloads must implement
pub trait TestWorkload: Send + Sync {
    /// Unique identifier for this test
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// Expected duration for test completion
    fn expected_duration(&self) -> Duration;

    /// Number of GPUs required per node (0 if no GPU requirement)
    fn required_gpus_per_node(&self) -> u32 {
        0 // default: no GPU requirement
    }

    /// Success criteria for validation
    fn success_criteria(&self) -> Vec<String>;

    /// Render the Kubernetes manifest for this test
    fn render_manifest(
        &self,
        test_id: &str,
        node_pair: &NodePair,
        config: &SelfTestConfig,
        rdma_info: &RdmaInfo,
    ) -> Result<String>;
}

/// Registry of all available test workloads
pub fn get_all_workloads() -> Vec<Box<dyn TestWorkload>> {
    vec![
        Box::new(nixl_transfer::NixlTransferTest),
        Box::new(deepgemm_minimal::DeepGemmMinimalTest),
        Box::new(deepgemm_simple::DeepGemmSimpleTest),
        Box::new(pplx_kernels::PplxKernelsTest),
        Box::new(deepep_internode::DeepEpInternodeTest),
        Box::new(deepep_low_latency::DeepEpLowLatencyTest),
    ]
}

/// Get a workload by name
pub fn get_workload_by_name(name: &str) -> Option<Box<dyn TestWorkload>> {
    get_all_workloads().into_iter().find(|w| w.name() == name)
}
