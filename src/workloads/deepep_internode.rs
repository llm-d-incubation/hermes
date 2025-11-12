use anyhow::Result;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepEpInternodeTest;

impl TestWorkload for DeepEpInternodeTest {
    fn name(&self) -> &str {
        "deepep-internode-test"
    }

    fn description(&self) -> &str {
        "DeepEP internode MoE expert parallel test across two nodes with RDMA"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(1200) // 20 minutes - test involves extensive tuning loops
    }

    fn required_gpus_per_node(&self) -> u32 {
        2 // supports 1, 2, 4, or 8 GPUs per node (default to 2 for flexibility)
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
        // build context using the unified template context
        let context = TemplateContext::new(test_id, node_pair, config, rdma_info)
            .with_embedded_files("05_deepep_internode");

        // render template with configured environment
        let template_str = include_str!("../../manifests/05_deepep_internode/manifest.yaml.j2");
        let mut env = super::create_template_environment();
        env.add_template("deepep_internode", template_str)?;
        let template = env.get_template("deepep_internode")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
