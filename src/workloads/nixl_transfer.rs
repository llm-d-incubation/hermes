use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;
use std::time::Duration;

use super::{RdmaInfo, TemplateNode, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct NixlTransferTest;

#[derive(Debug, Clone, Serialize)]
struct NixlTemplateContext {
    test_id: String,
    server_node: TemplateNode,
    client_node: TemplateNode,
    server_ip: String,
    rdma_resource_type: String,
    nixl_test_script: String,
    ucx_tls: String,
    ucx_gid_index: String,
    sriov_network: Option<String>,
    request_gpu: bool,
    image: String,
}

impl TestWorkload for NixlTransferTest {
    fn name(&self) -> &str {
        "nixl-transfer-test"
    }

    fn description(&self) -> &str {
        "Two-node NIXL data transfer test using UCX backend"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(180) // 3 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "NIXL agents initialized successfully".to_string(),
            "Memory registration completed".to_string(),
            "Agent metadata exchanged".to_string(),
            "Data transfer completed".to_string(),
        ]
    }

    fn render_manifest(
        &self,
        test_id: &str,
        node_pair: &NodePair,
        config: &SelfTestConfig,
        rdma_info: &RdmaInfo,
    ) -> Result<String> {
        // get first RDMA device from each node
        let server_rdma_device = node_pair
            .node1
            .rdma_interfaces
            .first()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "mlx5_0".to_string());

        let client_rdma_device = node_pair
            .node2
            .rdma_interfaces
            .first()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "mlx5_0".to_string());

        // use service DNS name for pod-to-pod communication
        let server_ip = format!("nixl-test-target.{}.svc.cluster.local", config.namespace);

        // load test script
        let nixl_test_script =
            include_str!("../../manifests/01_nixl_transfer/nixl-transfer-test.py");

        // indent script for YAML embedding
        let indented_script = nixl_test_script
            .lines()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("    {}", line)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let context = NixlTemplateContext {
            test_id: test_id.to_string(),
            server_node: TemplateNode {
                name: node_pair.node1.name.clone(),
                rdma_device: server_rdma_device,
            },
            client_node: TemplateNode {
                name: node_pair.node2.name.clone(),
                rdma_device: client_rdma_device,
            },
            server_ip,
            rdma_resource_type: rdma_info.rdma_resource_type.clone(),
            nixl_test_script: indented_script,
            ucx_tls: rdma_info.ucx_tls.clone(),
            ucx_gid_index: rdma_info.ucx_gid_index.clone(),
            sriov_network: rdma_info.sriov_network.clone(),
            request_gpu: config.request_gpu,
            image: config.image.clone(),
        };

        // render template
        let template_str = include_str!("../../manifests/01_nixl_transfer/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("nixl", template_str)?;
        let template = env.get_template("nixl")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
