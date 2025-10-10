use anyhow::Result;
use minijinja::Environment;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepGemmSimpleTest;

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
        rdma_info: &RdmaInfo,
    ) -> Result<String> {
        // build context using the unified template context
        let context = TemplateContext::new(test_id, node_pair, config, rdma_info)
            .with_embedded_files("03_deepgemm_simple");

        // render template
        let template_str = include_str!("../../manifests/03_deepgemm_simple/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("deepgemm_simple", template_str)?;
        let template = env.get_template("deepgemm_simple")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
