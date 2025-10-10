use anyhow::Result;
use minijinja::value::{Object, Value};
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
#[derive(Debug, Clone, Serialize)]
pub struct RdmaInfo {
    pub rdma_resource_type: String,
    pub sriov_network: Option<String>,
    pub ucx_tls: String,
    pub ucx_gid_index: String,
}

impl Object for RdmaInfo {
    fn get_value(self: &std::sync::Arc<Self>, key: &Value) -> Option<Value> {
        match key.as_str()? {
            "rdma_resource_type" => Some(Value::from(self.rdma_resource_type.clone())),
            "sriov_network" => Some(Value::from(self.sriov_network.clone())),
            "ucx_tls" => Some(Value::from(self.ucx_tls.clone())),
            "ucx_gid_index" => Some(Value::from(self.ucx_gid_index.clone())),
            _ => None,
        }
    }
}

/// Base template node info that most workloads need
#[derive(Debug, Clone, Serialize)]
pub struct TemplateNode {
    pub name: String,
    pub rdma_device: String,
}

/// Unified template context for all workloads
#[derive(Debug, Clone, Serialize)]
pub struct TemplateContext {
    pub test_id: String,
    pub server_node: crate::self_test::SelectedNode,
    pub client_node: crate::self_test::SelectedNode,
    pub selection_reason: String,
    pub rdma_resource_type: String,
    pub sriov_network: Option<String>,
    pub ucx_tls: String,
    pub ucx_gid_index: String,
    pub image: String,
    pub request_gpu: bool,
    pub gpu_count: u32,
    pub namespace: String,
    pub server_ip: String,
    pub extra_env_vars: std::collections::HashMap<String, String>,
    /// embedded files from manifest directory (filename -> base64 content)
    pub configmap_files: std::collections::HashMap<String, String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl TemplateContext {
    pub fn new(
        test_id: &str,
        node_pair: &crate::self_test::NodePair,
        config: &crate::self_test::SelfTestConfig,
        rdma_info: &RdmaInfo,
    ) -> Self {
        Self {
            test_id: test_id.to_string(),
            server_node: node_pair.node1.clone(),
            client_node: node_pair.node2.clone(),
            selection_reason: node_pair.selection_reason.clone(),
            rdma_resource_type: rdma_info.rdma_resource_type.clone(),
            sriov_network: rdma_info.sriov_network.clone(),
            ucx_tls: rdma_info.ucx_tls.clone(),
            ucx_gid_index: rdma_info.ucx_gid_index.clone(),
            image: config.image.clone(),
            request_gpu: config.gpu_requirement.requires_gpu(),
            gpu_count: config.gpus_per_node.unwrap_or(1),
            namespace: config.namespace.clone(),
            server_ip: format!("nixl-test-target.{}.svc.cluster.local", config.namespace),
            extra_env_vars: std::collections::HashMap::new(),
            configmap_files: std::collections::HashMap::new(),
            extra: std::collections::HashMap::new(),
        }
    }

    /// load embedded files for a workload and add to configmap_files
    pub fn with_embedded_files(mut self, workload_name: &str) -> Self {
        let files = crate::embedded_files::get_configmap_data(workload_name);

        if files.is_empty() {
            tracing::warn!(
                "No embedded files found for workload '{}'. ConfigMap will be empty. \
                 This may cause the workload to fail at runtime.",
                workload_name
            );
        } else {
            tracing::debug!(
                "Loaded {} embedded file(s) for workload '{}'",
                files.len(),
                workload_name
            );
        }

        self.configmap_files = files;
        self
    }

    /// add extra context variables
    pub fn with_extra(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// add environment variables
    pub fn with_env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env_vars.insert(key.into(), value.into());
        self
    }
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
