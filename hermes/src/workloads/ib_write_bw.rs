use std::time::Duration;

use super::TestWorkload;

pub struct IbWriteBwTest;

impl TestWorkload for IbWriteBwTest {
    fn name(&self) -> &str {
        "ib-write-bw-test"
    }

    fn description(&self) -> &str {
        "Two-node RDMA write bandwidth test using ib_write_bw"
    }

    fn expected_duration(&self) -> Duration {
        Duration::from_secs(120)
    }

    fn required_gpus_per_node(&self) -> u32 {
        0
    }

    fn default_image(&self) -> Option<&str> {
        Some("quay.io/wseaton/netdebug:latest")
    }
}
