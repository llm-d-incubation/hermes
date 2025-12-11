//! Test workload definitions for RDMA connectivity testing.
//!
//! Workloads are deployed via Helm charts in the `charts/` directory.
//! The TestWorkload trait defines metadata for each workload.

use serde::Serialize;
use std::time::Duration;

pub mod deepep_internode;
pub mod deepep_low_latency;
pub mod deepgemm_minimal;
pub mod deepgemm_simple;
pub mod ib_write_bw;
pub mod nixl_transfer;
pub mod pplx_kernels;

/// RDMA configuration info for display in manifest headers
#[derive(Debug, Clone, Serialize)]
pub struct RdmaInfo {
    pub rdma_resource_type: String,
    pub sriov_network: Option<String>,
    pub sriov_network_resource: Option<String>,
    pub ucx_tls: String,
    pub ucx_gid_index: String,
}

/// Trait that all test workloads must implement
pub trait TestWorkload: Send + Sync {
    /// Unique identifier for this test (must match Helm chart name in charts/)
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// Expected duration for test completion
    fn expected_duration(&self) -> Duration;

    /// Number of GPUs required per node (0 if no GPU requirement)
    fn required_gpus_per_node(&self) -> u32 {
        0
    }

    /// Default container image for this workload (overrides CLI default if Some)
    fn default_image(&self) -> Option<&str> {
        None
    }
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
        Box::new(ib_write_bw::IbWriteBwTest),
    ]
}

/// Get a workload by name
pub fn get_workload_by_name(name: &str) -> Option<Box<dyn TestWorkload>> {
    get_all_workloads().into_iter().find(|w| w.name() == name)
}
