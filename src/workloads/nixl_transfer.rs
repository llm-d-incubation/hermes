use anyhow::Result;
use minijinja::Environment;
use std::time::Duration;

use super::{RdmaInfo, TemplateContext, TestWorkload};
use crate::self_test::{NodePair, SelfTestConfig};

pub struct NixlTransferTest;

impl TestWorkload for NixlTransferTest {
    fn name(&self) -> &str {
        "nixl-transfer-test"
    }

    fn description(&self) -> &str {
        "Two-node NIXL data transfer test using UCX backend"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(180) // 3 minutes
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "NIXL agents initialized successfully".to_string(),
            "Memory registration completed".to_string(),
            "Agent metadata exchanged".to_string(),
            "Data transfer completed".to_string(),
        ]
    }

    fn render_manifest(
        &self,
        test_id: &str,
        node_pair: &NodePair,
        config: &SelfTestConfig,
        rdma_info: &RdmaInfo,
    ) -> Result<String> {
        // build context with embedded files
        let context = TemplateContext::new(test_id, node_pair, config, rdma_info)
            .with_embedded_files("01_nixl_transfer");

        // render template
        let template_str = include_str!("../../manifests/01_nixl_transfer/manifest.yaml.j2");
        let mut env = Environment::new();
        env.add_template("nixl", template_str)?;
        let template = env.get_template("nixl")?;
        let rendered = template.render(&context)?;

        Ok(rendered)
    }
}
