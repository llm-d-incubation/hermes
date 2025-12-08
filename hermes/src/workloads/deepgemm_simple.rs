use std::time::Duration;

use super::TestWorkload;

pub struct DeepGemmSimpleTest;

impl TestWorkload for DeepGemmSimpleTest {
    fn name(&self) -> &str {
        "deepgemm-simple-test"
    }

    fn description(&self) -> &str {
        "DeepGEMM simple FP8 GEMM and M-grouped tests on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(180)
    }
}
