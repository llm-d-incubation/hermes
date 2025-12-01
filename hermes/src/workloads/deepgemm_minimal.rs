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

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "DeepGEMM library imported successfully".to_string(),
            "CUDA available and working".to_string(),
            "FP8 tensor operations supported".to_string(),
        ]
    }
}
