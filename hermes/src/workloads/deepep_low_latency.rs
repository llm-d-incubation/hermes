use std::time::Duration;

use super::TestWorkload;

pub struct DeepEpLowLatencyTest;

impl TestWorkload for DeepEpLowLatencyTest {
    fn name(&self) -> &str {
        "deepep-lowlatency-test"
    }

    fn description(&self) -> &str {
        "DeepEP low latency MoE expert parallel test on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(600)
    }

    fn required_gpus_per_node(&self) -> u32 {
        1
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Repository cloned successfully".to_string(),
            "GPU detection successful".to_string(),
            "DeepEP low latency test completed".to_string(),
        ]
    }
}
