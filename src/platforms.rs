use crate::models::*;
use k8s_openapi::api::core::v1::Node;
use std::collections::BTreeMap;

pub trait PlatformDetector {
    fn detect_platform(&self, labels: &BTreeMap<String, String>) -> bool;
    fn get_platform_type(&self) -> PlatformType;
    fn detect_rdma_capability(&self, node: &Node) -> (bool, Option<String>, Option<String>);
    fn detect_topology_block(
        &self,
        node: &Node,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
    ) -> (Option<String>, Option<TopologyDetection>);
    fn extract_platform_specific_info(
        &self,
        node: &Node,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
    ) -> PlatformSpecificInfo;
}

#[derive(Debug, Default)]
pub struct PlatformSpecificInfo {
    // CoreWeave specific
    pub ib_speed: Option<String>,
    pub ib_fabric: Option<String>,
    pub ib_ports: Option<String>,
    pub leafgroup: Option<String>,
    pub superpod: Option<String>,
    pub neighbors: Vec<String>,

    // GKE specific
    pub gke_nodepool: Option<String>,
    pub gke_machine_family: Option<String>,
    pub gke_zone: Option<String>,
    pub gke_rdma_interfaces: Vec<GkeRdmaInterface>,
    pub gke_pci_topology: Option<String>,
    pub gke_fabric_domain: Option<String>,
    pub gke_topology_block: Option<String>,
    pub gke_topology_subblock: Option<String>,
    pub gke_topology_host: Option<String>,
}

pub struct CoreWeavePlatform;
pub struct OpenShiftPlatform;
pub struct GkePlatform;
pub struct GenericKubernetesPlatform;

impl PlatformDetector for CoreWeavePlatform {
    fn detect_platform(&self, labels: &BTreeMap<String, String>) -> bool {
        labels
            .iter()
            .any(|(k, _)| k.starts_with("ib.coreweave.cloud/"))
    }

    fn get_platform_type(&self) -> PlatformType {
        PlatformType::CoreWeave
    }

    fn detect_rdma_capability(&self, node: &Node) -> (bool, Option<String>, Option<String>) {
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let empty_labels = BTreeMap::new();
        let labels = node.metadata.labels.as_ref().unwrap_or(&empty_labels);

        if let Some(cap) = capacity {
            if cap.contains_key("rdma/roce_gdr") {
                let quantity = cap
                    .get("rdma/roce_gdr")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());

                // validate IB speed is not 0G
                if let Some(ib_speed) = labels.get("ib.coreweave.cloud/speed")
                    && ib_speed == "0G"
                {
                    return (false, None, None);
                }

                return (
                    true,
                    Some("RoCE GPU Direct".to_string()),
                    Some(format!("rdma/roce_gdr: {}", quantity)),
                );
            } else if cap.contains_key("rdma/ib") {
                let quantity = cap
                    .get("rdma/ib")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());

                // validate IB speed is not 0G
                if let Some(ib_speed) = labels.get("ib.coreweave.cloud/speed")
                    && ib_speed == "0G"
                {
                    return (false, None, None);
                }

                return (
                    true,
                    Some("InfiniBand".to_string()),
                    Some(format!("rdma/ib: {}", quantity)),
                );
            }
        }
        (false, None, None)
    }

    fn detect_topology_block(
        &self,
        _node: &Node,
        labels: &BTreeMap<String, String>,
        _annotations: &BTreeMap<String, String>,
    ) -> (Option<String>, Option<TopologyDetection>) {
        if let Some(leafgroup) = labels.get("ib.coreweave.cloud/leafgroup") {
            let detection = TopologyDetection {
                topology_type: TopologyType::LeafGroup,
                detection_method: "CoreWeave leafgroup label".to_string(),
                confidence: "High".to_string(),
            };
            (Some(leafgroup.clone()), Some(detection))
        } else {
            (None, None)
        }
    }

    fn extract_platform_specific_info(
        &self,
        _node: &Node,
        labels: &BTreeMap<String, String>,
        _annotations: &BTreeMap<String, String>,
    ) -> PlatformSpecificInfo {
        let neighbors = labels
            .iter()
            .filter_map(|(k, v)| {
                if k.starts_with("ib.coreweave.cloud/neighbors.current.ibp") {
                    Some(format!("{}={}", k, v))
                } else {
                    None
                }
            })
            .collect();

        PlatformSpecificInfo {
            ib_speed: labels.get("ib.coreweave.cloud/speed").cloned(),
            ib_fabric: labels.get("ib.coreweave.cloud/fabric").cloned(),
            ib_ports: labels.get("ib.coreweave.cloud/ports.current").cloned(),
            leafgroup: labels.get("ib.coreweave.cloud/leafgroup").cloned(),
            superpod: labels.get("ib.coreweave.cloud/superpod").cloned(),
            neighbors,
            ..Default::default()
        }
    }
}

impl PlatformDetector for OpenShiftPlatform {
    fn detect_platform(&self, labels: &BTreeMap<String, String>) -> bool {
        labels.contains_key("node.openshift.io/os_id")
    }

    fn get_platform_type(&self) -> PlatformType {
        PlatformType::OpenShift
    }

    fn detect_rdma_capability(&self, node: &Node) -> (bool, Option<String>, Option<String>) {
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let empty_labels = BTreeMap::new();
        let labels = node.metadata.labels.as_ref().unwrap_or(&empty_labels);

        if let Some(cap) = capacity {
            if cap.contains_key("rdma/roce_gdr") {
                let quantity = cap
                    .get("rdma/roce_gdr")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());
                return (
                    true,
                    Some("RoCE GPU Direct".to_string()),
                    Some(format!("rdma/roce_gdr: {}", quantity)),
                );
            } else if cap.contains_key("rdma/ib") {
                let quantity = cap
                    .get("rdma/ib")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());
                return (
                    true,
                    Some("InfiniBand".to_string()),
                    Some(format!("rdma/ib: {}", quantity)),
                );
            }
        }

        if labels
            .get("feature.node.kubernetes.io/rdma.capable")
            .map(|s| s.as_str())
            == Some("true")
        {
            return (true, Some("Generic RDMA".to_string()), None);
        }

        (false, None, None)
    }

    fn detect_topology_block(
        &self,
        node: &Node,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
    ) -> (Option<String>, Option<TopologyDetection>) {
        // prefer standard Kubernetes topology labels
        if let Some(zone) = labels.get("topology.kubernetes.io/zone") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "Kubernetes topology.kubernetes.io/zone label".to_string(),
                confidence: "High".to_string(),
            };
            return (Some(format!("zone-{}", zone)), Some(detection));
        }
        if let Some(zone) = labels.get("failure-domain.beta.kubernetes.io/zone") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "Kubernetes failure-domain.beta.kubernetes.io/zone label"
                    .to_string(),
                confidence: "High".to_string(),
            };
            return (Some(format!("zone-{}", zone)), Some(detection));
        }

        // fallback to IP-based topology
        if let Some(ip_pattern) = extract_ip_topology_block(node, annotations) {
            let detection = TopologyDetection {
                topology_type: if ip_pattern.starts_with("ip-range") {
                    TopologyType::IpRange
                } else {
                    TopologyType::Subnet
                },
                detection_method: "IP address pattern analysis".to_string(),
                confidence: "Medium".to_string(),
            };
            return (Some(ip_pattern), Some(detection));
        }

        (None, None)
    }

    fn extract_platform_specific_info(
        &self,
        _node: &Node,
        _labels: &BTreeMap<String, String>,
        _annotations: &BTreeMap<String, String>,
    ) -> PlatformSpecificInfo {
        PlatformSpecificInfo::default()
    }
}

impl PlatformDetector for GkePlatform {
    fn detect_platform(&self, labels: &BTreeMap<String, String>) -> bool {
        labels
            .iter()
            .any(|(k, _)| k.starts_with("cloud.google.com/gke-"))
    }

    fn get_platform_type(&self) -> PlatformType {
        PlatformType::GKE
    }

    fn detect_rdma_capability(&self, node: &Node) -> (bool, Option<String>, Option<String>) {
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let empty_labels = BTreeMap::new();
        let labels = node.metadata.labels.as_ref().unwrap_or(&empty_labels);

        if let Some(cap) = capacity {
            // GKE RDMA detection via networking.gke.io.networks resources
            if cap
                .iter()
                .any(|(k, _)| k.starts_with("networking.gke.io.networks/rdma-"))
            {
                let rdma_count = cap
                    .iter()
                    .filter(|(k, _)| {
                        k.starts_with("networking.gke.io.networks/rdma-") && !k.ends_with(".IP")
                    })
                    .count();
                return (
                    true,
                    Some("GKE RDMA".to_string()),
                    Some(format!("{} RDMA interfaces", rdma_count)),
                );
            }
            // GKE gVNIC detection
            else if cap
                .iter()
                .any(|(k, _)| k.starts_with("networking.gke.io.networks/gvnic-"))
            {
                let gvnic_count = cap
                    .iter()
                    .filter(|(k, _)| {
                        k.starts_with("networking.gke.io.networks/gvnic-") && !k.ends_with(".IP")
                    })
                    .count();
                return (
                    true,
                    Some("gVNIC (Google Virtual NIC)".to_string()),
                    Some(format!("{} gVNIC interfaces", gvnic_count)),
                );
            }
        }

        // check for gVNIC support via labels
        if labels.get("cloud.google.com/gke-gvnic").map(|s| s.as_str()) == Some("true") {
            return (
                true,
                Some("gVNIC (Google Virtual NIC)".to_string()),
                Some("gVNIC enabled".to_string()),
            );
        }

        (false, None, None)
    }

    fn detect_topology_block(
        &self,
        _node: &Node,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
    ) -> (Option<String>, Option<TopologyDetection>) {
        // prioritize GKE topology block labels for RDMA fabric grouping
        if let Some(topology_block) = labels.get("cloud.google.com/gce-topology-block") {
            let detection = TopologyDetection {
                topology_type: TopologyType::GkeBlock,
                detection_method: "GKE cloud.google.com/gce-topology-block label".to_string(),
                confidence: "High".to_string(),
            };
            return (
                Some(format!("block-{}", &topology_block[..8])),
                Some(detection),
            );
        }

        // fallback to subblock if block not available
        if let Some(topology_subblock) = labels.get("cloud.google.com/gce-topology-subblock") {
            let detection = TopologyDetection {
                topology_type: TopologyType::GkeBlock,
                detection_method: "GKE cloud.google.com/gce-topology-subblock label".to_string(),
                confidence: "Medium".to_string(),
            };
            return (
                Some(format!("subblock-{}", &topology_subblock[..8])),
                Some(detection),
            );
        }

        // for GPU nodes, detect fabric domains from RDMA network analysis
        if let Some(fabric_domain) = extract_gke_fabric_domain(annotations) {
            let detection = TopologyDetection {
                topology_type: TopologyType::Hardware,
                detection_method: "GKE RDMA fabric domain analysis".to_string(),
                confidence: "High".to_string(),
            };
            return (Some(fabric_domain), Some(detection));
        }

        // fallback to zone+nodepool
        if let (Some(zone), Some(nodepool)) = (
            labels
                .get("topology.gke.io/zone")
                .or_else(|| labels.get("topology.kubernetes.io/zone")),
            labels.get("cloud.google.com/gke-nodepool"),
        ) {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "GKE zone+nodepool topology".to_string(),
                confidence: "Medium".to_string(),
            };
            (Some(format!("{}-{}", zone, nodepool)), Some(detection))
        } else if let Some(zone) = labels.get("topology.gke.io/zone") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "GKE topology.gke.io/zone label".to_string(),
                confidence: "Medium".to_string(),
            };
            (Some(format!("gke-zone-{}", zone)), Some(detection))
        } else {
            (None, None)
        }
    }

    fn extract_platform_specific_info(
        &self,
        _node: &Node,
        labels: &BTreeMap<String, String>,
        annotations: &BTreeMap<String, String>,
    ) -> PlatformSpecificInfo {
        let (gke_rdma_interfaces, gke_pci_topology, gke_fabric_domain) =
            parse_gke_networking_info(annotations);

        PlatformSpecificInfo {
            gke_nodepool: labels.get("cloud.google.com/gke-nodepool").cloned(),
            gke_machine_family: labels.get("cloud.google.com/machine-family").cloned(),
            gke_zone: labels.get("topology.gke.io/zone").cloned(),
            gke_topology_block: labels.get("cloud.google.com/gce-topology-block").cloned(),
            gke_topology_subblock: labels
                .get("cloud.google.com/gce-topology-subblock")
                .cloned(),
            gke_topology_host: labels.get("cloud.google.com/gce-topology-host").cloned(),
            gke_rdma_interfaces,
            gke_pci_topology,
            gke_fabric_domain,
            ..Default::default()
        }
    }
}

impl PlatformDetector for GenericKubernetesPlatform {
    fn detect_platform(&self, _labels: &BTreeMap<String, String>) -> bool {
        true // fallback for everything else
    }

    fn get_platform_type(&self) -> PlatformType {
        PlatformType::GenericKubernetes
    }

    fn detect_rdma_capability(&self, node: &Node) -> (bool, Option<String>, Option<String>) {
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let empty_labels = BTreeMap::new();
        let labels = node.metadata.labels.as_ref().unwrap_or(&empty_labels);

        if let Some(cap) = capacity {
            if cap.contains_key("rdma/roce_gdr") {
                let quantity = cap
                    .get("rdma/roce_gdr")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());
                return (
                    true,
                    Some("RoCE GPU Direct".to_string()),
                    Some(format!("rdma/roce_gdr: {}", quantity)),
                );
            } else if cap.contains_key("rdma/ib") {
                let quantity = cap
                    .get("rdma/ib")
                    .map(|q| format!("{:?}", q))
                    .unwrap_or_else(|| "unknown".to_string());
                return (
                    true,
                    Some("InfiniBand".to_string()),
                    Some(format!("rdma/ib: {}", quantity)),
                );
            }
        }

        if labels
            .get("feature.node.kubernetes.io/rdma.capable")
            .map(|s| s.as_str())
            == Some("true")
        {
            return (true, Some("Generic RDMA".to_string()), None);
        }

        (false, None, None)
    }

    fn detect_topology_block(
        &self,
        _node: &Node,
        labels: &BTreeMap<String, String>,
        _annotations: &BTreeMap<String, String>,
    ) -> (Option<String>, Option<TopologyDetection>) {
        if let Some(zone) = labels.get("topology.kubernetes.io/zone") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "Kubernetes topology.kubernetes.io/zone label".to_string(),
                confidence: "High".to_string(),
            };
            (Some(format!("zone-{}", zone)), Some(detection))
        } else if let Some(zone) = labels.get("failure-domain.beta.kubernetes.io/zone") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Zone,
                detection_method: "Kubernetes failure-domain.beta.kubernetes.io/zone label"
                    .to_string(),
                confidence: "High".to_string(),
            };
            (Some(format!("zone-{}", zone)), Some(detection))
        } else if let Some(rack) = labels.get("topology.kubernetes.io/rack") {
            let detection = TopologyDetection {
                topology_type: TopologyType::Rack,
                detection_method: "Kubernetes topology.kubernetes.io/rack label".to_string(),
                confidence: "High".to_string(),
            };
            (Some(format!("rack-{}", rack)), Some(detection))
        } else {
            (None, None)
        }
    }

    fn extract_platform_specific_info(
        &self,
        _node: &Node,
        _labels: &BTreeMap<String, String>,
        _annotations: &BTreeMap<String, String>,
    ) -> PlatformSpecificInfo {
        PlatformSpecificInfo::default()
    }
}

pub fn detect_platform_from_labels(labels: &BTreeMap<String, String>) -> Box<dyn PlatformDetector> {
    let detectors: Vec<Box<dyn PlatformDetector>> = vec![
        Box::new(OpenShiftPlatform),
        Box::new(CoreWeavePlatform),
        Box::new(GkePlatform),
        Box::new(GenericKubernetesPlatform),
    ];

    for detector in detectors {
        if detector.detect_platform(labels) {
            return detector;
        }
    }

    // fallback to generic (this should never happen since GenericKubernetesPlatform always returns true)
    Box::new(GenericKubernetesPlatform)
}

// Helper functions that need to be accessible from platform implementations
fn extract_ip_topology_block(
    node: &Node,
    annotations: &BTreeMap<String, String>,
) -> Option<String> {
    // look for internal IP address patterns
    if let Some(addresses) = node.status.as_ref().and_then(|s| s.addresses.as_ref()) {
        for addr in addresses {
            if addr.type_ == "InternalIP" {
                let parts: Vec<&str> = addr.address.split('.').collect();
                if parts.len() >= 4
                    && let Ok(last_octet) = parts[3].parse::<u32>()
                {
                    let group = last_octet / 10;
                    return Some(format!(
                        "ip-range-{}.{}.{}.{}-{}",
                        parts[0],
                        parts[1],
                        parts[2],
                        group * 10,
                        (group + 1) * 10 - 1
                    ));
                }
            }
        }
    }

    // fallback to transit IP
    if let Some(transit_ip_json) = annotations.get("k8s.ovn.org/node-transit-switch-port-ifaddr")
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(transit_ip_json)
        && let Some(ipv4) = parsed.get("ipv4").and_then(|v| v.as_str())
        && let Some(ip_part) = ipv4.split('/').next()
    {
        let parts: Vec<&str> = ip_part.split('.').collect();
        if parts.len() >= 3 {
            return Some(format!("subnet-{}.{}.{}", parts[0], parts[1], parts[2]));
        }
    }
    None
}

fn extract_gke_fabric_domain(annotations: &BTreeMap<String, String>) -> Option<String> {
    if let Some(networks_json) = annotations.get("networking.gke.io/networks")
        && let Ok(networks) = serde_json::from_str::<Vec<serde_json::Value>>(networks_json)
    {
        for network in networks {
            if let (Some(name), Some(cidrs)) = (
                network.get("name").and_then(|v| v.as_str()),
                network.get("cidrs").and_then(|v| v.as_array()),
            ) && name == "rdma-0"
                && !cidrs.is_empty()
                && let Some(cidr) = cidrs[0].as_str()
                && let Some(subnet) = cidr.split('/').next().and_then(|ip| {
                    let parts: Vec<&str> = ip.split('.').collect();
                    if parts.len() >= 3 {
                        Some(format!("{}.{}.{}", parts[0], parts[1], parts[2]))
                    } else {
                        None
                    }
                })
            {
                return Some(format!("fabric-{}", subnet));
            }
        }
    }
    None
}

fn parse_gke_networking_info(
    annotations: &BTreeMap<String, String>,
) -> (Vec<GkeRdmaInterface>, Option<String>, Option<String>) {
    let mut rdma_interfaces = Vec::new();
    let mut pci_topology = None;

    // parse networking.gke.io/nic-info for detailed interface info
    if let Some(nic_info_json) = annotations.get("networking.gke.io/nic-info")
        && let Ok(nic_info) = serde_json::from_str::<Vec<serde_json::Value>>(nic_info_json)
    {
        for nic in nic_info {
            if let (Some(birth_name), Some(pci_address), Some(birth_ip)) = (
                nic.get("birthName").and_then(|v| v.as_str()),
                nic.get("pciAddress").and_then(|v| v.as_str()),
                nic.get("birthIP").and_then(|v| v.as_str()),
            ) {
                // only include RDMA interfaces (gpu*rdma*)
                if birth_name.contains("rdma") {
                    let network_name =
                        if birth_name.starts_with("gpu") && birth_name.contains("rdma") {
                            // extract gpu number from gpu0rdma0 -> rdma-0
                            let gpu_num = birth_name.chars().nth(3).unwrap_or('0');
                            format!("rdma-{}", gpu_num)
                        } else {
                            birth_name.to_string()
                        };

                    // extract subnet from IP (192.168.0.x -> 192.168.0)
                    let subnet = birth_ip
                        .rsplit_once('.')
                        .map(|x| x.0)
                        .unwrap_or(birth_ip)
                        .to_string();

                    rdma_interfaces.push(GkeRdmaInterface {
                        network_name,
                        pci_address: pci_address.to_string(),
                        birth_name: birth_name.to_string(),
                        ip_address: birth_ip.to_string(),
                        subnet,
                    });
                }
            }
        }

        // create PCI topology grouping from the addresses
        if !rdma_interfaces.is_empty() {
            let pci_buses: Vec<String> = rdma_interfaces
                .iter()
                .map(|iface| {
                    // extract bus from 0000:91:00.0 -> 91
                    iface
                        .pci_address
                        .split(':')
                        .nth(1)
                        .unwrap_or("00")
                        .to_string()
                })
                .collect();

            let unique_buses: std::collections::HashSet<_> = pci_buses.iter().collect();
            pci_topology = Some(format!("pci-buses-{}", unique_buses.len()));
        }
    }

    // detect fabric domain from RDMA network subnets
    let fabric_domain = extract_gke_fabric_domain(annotations);

    (rdma_interfaces, pci_topology, fabric_domain)
}
