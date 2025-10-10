use anyhow::Result;
use minijinja::Environment;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct PplxKernelsTest;

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
        // build context using the unified template context
        let context = TemplateContext::new(test_id, node_pair, config, rdma_info);

        // render template
        let template_str = include_str!("../../manifests/04_pplx_kernels/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("pplx_kernels", template_str)?;
        let template = env.get_template("pplx_kernels")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
