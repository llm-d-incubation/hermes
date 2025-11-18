use anyhow::{Context, Result};
use kube::{Api, Client};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::crds::nvidia_network::NicClusterPolicy;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdmaDevicePluginConfig {
    pub config_list: Vec<RdmaDeviceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdmaDeviceConfig {
    pub resource_name: String,
    #[serde(default)]
    pub rdma_hca_max: Option<u32>,
    #[serde(default)]
    pub selectors: Option<RdmaDeviceSelectors>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdmaDeviceSelectors {
    #[serde(default)]
    pub vendors: Option<Vec<String>>,
    #[serde(default)]
    pub device_i_ds: Option<Vec<String>>,
    #[serde(default)]
    pub if_names: Option<Vec<String>>,
}

/// detect NVIDIA Network Operator and extract RDMA resource configurations
pub async fn detect_nvidia_network_operator(client: &Client) -> Result<Option<Vec<String>>> {
    let api: Api<NicClusterPolicy> = Api::all(client.clone());

    // try to list NicClusterPolicies
    let policies = match api.list(&Default::default()).await {
        Ok(list) => list,
        Err(e) => {
            debug!(
                "NVIDIA Network Operator not detected (could not list NicClusterPolicies): {}",
                e
            );
            return Ok(None);
        }
    };

    if policies.items.is_empty() {
        debug!("No NicClusterPolicies found");
        return Ok(None);
    }

    let mut resource_names = Vec::new();

    for policy in &policies.items {
        let policy_name = policy.metadata.name.as_deref().unwrap_or("unknown");

        debug!("Found NicClusterPolicy: {}", policy_name);

        // extract RDMA shared device plugin config
        if let Some(rdma_plugin) = &policy.spec.rdma_shared_device_plugin {
            if let Some(config_json) = &rdma_plugin.config {
                match parse_rdma_device_plugin_config(config_json) {
                    Ok(config) => {
                        for device_config in config.config_list {
                            debug!(
                                "Found RDMA resource: {} (policy: {})",
                                device_config.resource_name, policy_name
                            );
                            resource_names.push(device_config.resource_name);
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse RDMA device plugin config for policy {}: {}",
                            policy_name, e
                        );
                    }
                }
            }
        }

        // also check SR-IOV device plugin if configured
        if let Some(sriov_plugin) = &policy.spec.sriov_device_plugin {
            if let Some(config_json) = &sriov_plugin.config {
                match parse_rdma_device_plugin_config(config_json) {
                    Ok(config) => {
                        for device_config in config.config_list {
                            debug!(
                                "Found SR-IOV RDMA resource: {} (policy: {})",
                                device_config.resource_name, policy_name
                            );
                            resource_names.push(device_config.resource_name);
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse SR-IOV device plugin config for policy {}: {}",
                            policy_name, e
                        );
                    }
                }
            }
        }
    }

    if resource_names.is_empty() {
        Ok(None)
    } else {
        Ok(Some(resource_names))
    }
}

fn parse_rdma_device_plugin_config(config_json: &str) -> Result<RdmaDevicePluginConfig> {
    serde_json::from_str(config_json).context("Failed to parse RDMA device plugin config JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rdma_config() {
        let config_json = r#"{
  "configList": [
      {
          "rdmaHcaMax": 1000,
          "resourceName": "roce_gdr",
          "selectors": {
            "vendors": ["15b3"],
            "deviceIDs": ["101d", "0063"],
            "ifNames": ["p0", "p1", "p2", "p3"]
          }
      }
  ]
}"#;

        let config = parse_rdma_device_plugin_config(config_json).unwrap();
        assert_eq!(config.config_list.len(), 1);
        assert_eq!(config.config_list[0].resource_name, "roce_gdr");
        assert_eq!(config.config_list[0].rdma_hca_max, Some(1000));
    }

    #[test]
    fn test_parse_multiple_resources() {
        let config_json = r#"{
  "configList": [
      {
          "resourceName": "rdma_ib",
          "rdmaHcaMax": 500
      },
      {
          "resourceName": "rdma_roce",
          "rdmaHcaMax": 1000
      }
  ]
}"#;

        let config = parse_rdma_device_plugin_config(config_json).unwrap();
        assert_eq!(config.config_list.len(), 2);
        assert_eq!(config.config_list[0].resource_name, "rdma_ib");
        assert_eq!(config.config_list[1].resource_name, "rdma_roce");
    }

    #[test]
    fn test_parse_real_openshift_config() {
        // actual config from OpenShift cluster with NVIDIA Network Operator
        let config_json = r#"{
  "configList": [
      {
          "rdmaHcaMax": 1000,
          "resourceName": "roce_gdr",
          "selectors": {
            "vendors": ["15b3"],
            "deviceIDs": ["101d", "0063" ],
            "ifNames": ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13", "p14", "p15", "q0", "q1"]
          }
      }
  ]
}"#;

        let config = parse_rdma_device_plugin_config(config_json).unwrap();
        assert_eq!(config.config_list.len(), 1);
        assert_eq!(config.config_list[0].resource_name, "roce_gdr");
        assert_eq!(config.config_list[0].rdma_hca_max, Some(1000));

        let selectors = config.config_list[0].selectors.as_ref().unwrap();
        assert_eq!(selectors.vendors, Some(vec!["15b3".to_string()]));
        assert!(selectors.if_names.is_some());
        assert_eq!(selectors.if_names.as_ref().unwrap().len(), 18);
    }
}
