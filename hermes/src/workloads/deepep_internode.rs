use std::time::Duration;

use super::TestWorkload;

pub struct DeepEpInternodeTest;

impl TestWorkload for DeepEpInternodeTest {
    fn name(&self) -> &str {
        "deepep-internode-test"
    }

    fn description(&self) -> &str {
        "DeepEP internode MoE expert parallel test across two nodes with RDMA"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(1200)
    }

    fn required_gpus_per_node(&self) -> u32 {
        2
    }
}
