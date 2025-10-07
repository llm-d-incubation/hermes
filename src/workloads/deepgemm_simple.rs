use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;
use std::time::Duration;

use super::{RdmaInfo, TemplateNode, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepGemmSimpleTest;

#[derive(Debug, Clone, Serialize)]
struct DeepGemmSimpleTemplateContext {
    test_id: String,
    server_node: TemplateNode,
    client_node: TemplateNode,
    deepgemm_simple_test_script: String,
    image: String,
}

impl TestWorkload for DeepGemmSimpleTest {
    fn name(&self) -> &str {
        "deepgemm-simple-test"
    }

    fn description(&self) -> &str {
        "DeepGEMM simple FP8 GEMM and M-grouped tests on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(180) // 3 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Library import successful".to_string(),
            "Basic FP8 GEMM test passed".to_string(),
            "M-grouped FP8 GEMM test passed".to_string(),
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
            include_str!("../../manifests/03_deepgemm_simple/deepgemm-simple-test.py");

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

        let context = DeepGemmSimpleTemplateContext {
            test_id: test_id.to_string(),
            server_node: TemplateNode {
                name: node_pair.node1.name.clone(),
                rdma_device: server_rdma_device,
            },
            client_node: TemplateNode {
                name: node_pair.node2.name.clone(),
                rdma_device: client_rdma_device,
            },
            deepgemm_simple_test_script: indented_script,
            image: config.image.clone(),
        };

        // render template
        let template_str = include_str!("../../manifests/03_deepgemm_simple/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("deepgemm_simple", template_str)?;
        let template = env.get_template("deepgemm_simple")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
