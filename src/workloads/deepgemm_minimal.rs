use anyhow::Result;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepGemmMinimalTest;

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
        rdma_info: &RdmaInfo,
    ) -> Result<String> {
        // build context using the unified template context
        let context = TemplateContext::new(test_id, node_pair, config, rdma_info)
            .with_embedded_files("02_deepgemm_minimal");

        // render template with configured environment
        let template_str = include_str!("../../manifests/02_deepgemm_minimal/manifest.yaml.j2");
        let mut env = super::create_template_environment();
        env.add_template("deepgemm", template_str)?;
        let template = env.get_template("deepgemm")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
