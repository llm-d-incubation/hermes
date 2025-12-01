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

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Library import successful".to_string(),
            "Basic FP8 GEMM test passed".to_string(),
            "M-grouped FP8 GEMM test passed".to_string(),
        ]
    }
}
