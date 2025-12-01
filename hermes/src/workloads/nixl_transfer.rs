use std::time::Duration;

use super::TestWorkload;

pub struct NixlTransferTest;

impl TestWorkload for NixlTransferTest {
    fn name(&self) -> &str {
        "nixl-transfer-test"
    }

    fn description(&self) -> &str {
        "Two-node NIXL data transfer test using UCX backend"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(180)
    }

    fn success_criteria(&self) -> Vec<String> {
        vec![
            "NIXL agents initialized successfully".to_string(),
            "Memory registration completed".to_string(),
            "Agent metadata exchanged".to_string(),
            "Data transfer completed".to_string(),
        ]
    }
}
