use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;
use std::time::Duration;

use super::{RdmaInfo, TemplateNode, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct PplxKernelsTest;

#[derive(Debug, Clone, Serialize)]
struct PplxKernelsTemplateContext {
    test_id: String,
    server_node: TemplateNode,
    client_node: TemplateNode,
    rdma_resource_type: String,
    image: String,
}

impl TestWorkload for PplxKernelsTest {
    fn name(&self) -> &str {
        "pplx-kernels-test"
    }

    fn description(&self) -> &str {
        "pplx-kernels all-to-all communication benchmark on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Repository cloned successfully".to_string(),
            "Dependencies installed".to_string(),
            "All-to-all benchmark completed".to_string(),
        ]
    }

    fn render_manifest(
        &self,
        test_id: &str,
        node_pair: &NodePair,
        config: &SelfTestConfig,
        rdma_info: &RdmaInfo,
    ) -> Result<String> {
        let server_rdma_device = "none".to_string();
        let client_rdma_device = "none".to_string();

        let context = PplxKernelsTemplateContext {
            test_id: test_id.to_string(),
            server_node: TemplateNode {
                name: node_pair.node1.name.clone(),
                rdma_device: server_rdma_device,
            },
            client_node: TemplateNode {
                name: node_pair.node2.name.clone(),
                rdma_device: client_rdma_device,
            },
            rdma_resource_type: rdma_info.rdma_resource_type.clone(),
            image: config.image.clone(),
        };

        // render template
        let template_str = include_str!("../../manifests/04_pplx_kernels/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("pplx_kernels", template_str)?;
        let template = env.get_template("pplx_kernels")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
