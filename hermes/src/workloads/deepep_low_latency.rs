use anyhow::Result;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct DeepEpLowLatencyTest;

impl TestWorkload for DeepEpLowLatencyTest {
    fn name(&self) -> &str {
        "deepep-lowlatency-test"
    }

    fn description(&self) -> &str {
        "DeepEP low latency MoE expert parallel test on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(600) // 10 minutes
    }

    fn required_gpus_per_node(&self) -> u32 {
        1 // internode test requires at least 1 local rank (1 GPU) per node
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Repository cloned successfully".to_string(),
            "GPU detection successful".to_string(),
            "DeepEP low latency test completed".to_string(),
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
            .with_embedded_files("06_deepep_low_latency")
            .with_active_deadline(self.expected_duration());

        // render template with configured environment
        let template_str =
            include_str!("../../../manifests/06_deepep_low_latency/manifest.yaml.j2");
        let mut env = super::create_template_environment();
        env.add_template("deepep_low_latency", template_str)?;
        let template = env.get_template("deepep_low_latency")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
