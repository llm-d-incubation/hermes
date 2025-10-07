use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;
use std::time::Duration;

use super::{RdmaInfo, TemplateNode, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepEpInternodeTest;

#[derive(Debug, Clone, Serialize)]
struct DeepEpInternodeTemplateContext {
    test_id: String,
    server_node: TemplateNode,
    client_node: TemplateNode,
    rdma_resource_type: String,
    image: String,
}

impl TestWorkload for DeepEpInternodeTest {
    fn name(&self) -> &str {
        "deepep-internode-test"
    }

    fn description(&self) -> &str {
        "DeepEP internode MoE expert parallel test across two nodes with RDMA"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(600) // 10 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Repository cloned successfully".to_string(),
            "GPU detection successful".to_string(),
            "DeepEP internode test completed".to_string(),
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

        let context = DeepEpInternodeTemplateContext {
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
        let template_str = include_str!("../../manifests/05_deepep_internode/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("deepep_internode", template_str)?;
        let template = env.get_template("deepep_internode")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
