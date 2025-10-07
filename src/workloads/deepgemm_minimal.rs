use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;
use std::time::Duration;

use super::{RdmaInfo, TemplateNode, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepGemmMinimalTest;

#[derive(Debug, Clone, Serialize)]
struct DeepGemmMinimalTemplateContext {
    test_id: String,
    server_node: TemplateNode,
    client_node: TemplateNode,
    deepgemm_minimal_test_script: String,
    image: String,
}

impl TestWorkload for DeepGemmMinimalTest {
    fn name(&self) -> &str {
        "deepgemm-minimal-test"
    }

    fn description(&self) -> &str {
        "DeepGEMM library availability test on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(120) // 2 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "DeepGEMM library imported successfully".to_string(),
            "CUDA available and working".to_string(),
            "FP8 tensor operations supported".to_string(),
        ]
    }

    fn render_manifest(
        &self,
        test_id: &str,
        node_pair: &NodePair,
        config: &SelfTestConfig,
        _rdma_info: &RdmaInfo,
    ) -> Result<String> {
        // deepgemm test doesn't need RDMA devices, just GPU
        let server_rdma_device = "none".to_string();
        let client_rdma_device = "none".to_string();

        // load test script
        let test_script =
            include_str!("../../manifests/02_deepgemm_minimal/deepgemm-minimal-test.py");

        // indent script for YAML embedding
        let indented_script = test_script
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

        let context = DeepGemmMinimalTemplateContext {
            test_id: test_id.to_string(),
            server_node: TemplateNode {
                name: node_pair.node1.name.clone(),
                rdma_device: server_rdma_device,
            },
            client_node: TemplateNode {
                name: node_pair.node2.name.clone(),
                rdma_device: client_rdma_device,
            },
            deepgemm_minimal_test_script: indented_script,
            image: config.image.clone(),
        };

        // render template
        let template_str = include_str!("../../manifests/02_deepgemm_minimal/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("deepgemm", template_str)?;
        let template = env.get_template("deepgemm")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
