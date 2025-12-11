use std::time::Duration;

use super::TestWorkload;

pub struct DeepGemmMinimalTest;

impl TestWorkload for DeepGemmMinimalTest {
    fn name(&self) -> &str {
        "deepgemm-minimal-test"
    }

    fn description(&self) -> &str {
        "DeepGEMM library availability test on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(120)
    }
}
