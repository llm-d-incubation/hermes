use std::time::Duration;

use super::TestWorkload;

pub struct PplxKernelsTest;

impl TestWorkload for PplxKernelsTest {
    fn name(&self) -> &str {
        "pplx-kernels-test"
    }

    fn description(&self) -> &str {
        "pplx-kernels all-to-all communication benchmark on two nodes"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(300)
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "Repository cloned successfully".to_string(),
            "Dependencies installed".to_string(),
            "All-to-all benchmark completed".to_string(),
        ]
    }
}
