use anyhow::Result;
use minijinja::Environment;
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
        Duration::from_secs(240) // 4 minutes
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
            .with_embedded_files("06_deepep_low_latency");

        // render template
        let template_str = include_str!("../../manifests/06_deepep_low_latency/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("deepep_low_latency", template_str)?;
        let template = env.get_template("deepep_low_latency")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
