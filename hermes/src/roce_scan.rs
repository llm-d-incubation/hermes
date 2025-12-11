use anyhow::{Context, Result};
use k8s_openapi::api::core::v1::{
    Container, HostPathVolumeSource, Node, Pod, PodSpec, Volume, VolumeMount,
};
use kube::{
    Api, Client, Config,
    api::{DeleteParams, ListParams, PostParams},
};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::analyzer::ClusterAnalyzer;
use crate::formatters::get_formatter;
use crate::models::{
    ClusterReport, LabelDetailLevel, NamespaceRoceConfig, NamespaceType, NodeInfo, PlatformType,
    RoceConfig,
};
use crate::platforms::detect_platform_from_labels;

/// Configuration for scan-roce command
pub struct RoceScanConfig {
    pub format: String,
    pub rdma_only: bool,
    pub image: String,
    pub namespace: String,
    pub cleanup: bool,
    pub topology_rule: Option<String>,
    pub detailed_labels: bool,
    pub save_to: Option<String>,
}

/// Multi-namespace detection output from pod
#[derive(Debug, Deserialize)]
struct MultiNamespaceOutput {
    namespaces: Vec<NamespaceDetectionResult>,
}

/// Single namespace detection result from pod
#[derive(Debug, Deserialize)]
struct NamespaceDetectionResult {
    namespace_type: String,
    namespace_id: String,
    #[serde(default)]
    pod_name: Option<String>,
    #[serde(default)]
    pod_namespace: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    config: RoceConfig,
}

/// Run the complete scan-roce workflow
pub async fn run_roce_scan(config: RoceScanConfig) -> Result<()> {
    info!("Starting RoCE-aware cluster scan...");

    // setup kubernetes client with proxy support
    let (client, kube_config) = setup_client().await?;

    // step 1: perform regular cluster scan
    info!("Performing initial cluster scan...");
    let mut cluster_report = perform_cluster_scan(&client, &kube_config, &config).await?;

    // step 2: filter to RDMA-capable nodes if requested
    let rdma_nodes: Vec<&NodeInfo> = cluster_report
        .nodes
        .iter()
        .filter(|n| n.rdma_capability.is_capable())
        .collect();

    if rdma_nodes.is_empty() {
        warn!("No RDMA-capable nodes found in cluster");
        output_report(&cluster_report, &config)?;
        return Ok(());
    }

    info!("Found {} RDMA-capable nodes", rdma_nodes.len());

    // step 3: deploy RoCE detection pods on each RDMA node
    info!(
        "Deploying RoCE detection pods on {} nodes...",
        rdma_nodes.len()
    );
    let pod_deployments =
        deploy_roce_detection_pods(&client, &config.namespace, &config.image, &rdma_nodes).await?;

    // step 4: wait for all pods to complete
    info!("Waiting for RoCE detection to complete...");
    let pod_results = wait_for_pods_completion(
        &client,
        &config.namespace,
        &pod_deployments,
        Duration::from_secs(120),
    )
    .await?;

    // step 5: collect logs and parse RoCE config
    info!("Collecting RoCE configuration from pods...");
    let roce_configs = collect_roce_configs(&client, &config.namespace, &pod_results).await?;

    // step 6: merge RoCE configs into cluster report
    merge_roce_configs_into_report(&mut cluster_report, &roce_configs);

    // step 7: cleanup pods if requested
    if config.cleanup {
        info!("Cleaning up RoCE detection pods...");
        cleanup_pods(&client, &config.namespace, &pod_deployments).await?;
    } else {
        info!("Detection pods preserved (use --cleanup to remove)");
    }

    // step 8: output results
    output_report(&cluster_report, &config)?;

    Ok(())
}

/// Setup Kubernetes client with proxy support
async fn setup_client() -> Result<(Client, Config)> {
    if let Ok(proxy) = std::env::var("HTTPS_PROXY") {
        info!("Using proxy: {}", proxy);
        let mut config = Config::infer().await?;

        if std::env::var("KUBE_INSECURE_TLS").is_ok()
            || std::env::var("KUBERNETES_INSECURE_TLS").is_ok()
        {
            info!("Disabling TLS certificate verification due to environment variable");
            config.accept_invalid_certs = true;
        }

        Ok((Client::try_from(config.clone())?, config))
    } else {
        let config = Config::infer().await?;
        Ok((Client::try_from(config.clone())?, config))
    }
}

/// Perform initial cluster scan without RoCE detection
async fn perform_cluster_scan(
    client: &Client,
    kube_config: &Config,
    config: &RoceScanConfig,
) -> Result<ClusterReport> {
    let nodes: Api<Node> = Api::all(client.clone());
    let node_list = nodes.list(&ListParams::default()).await?;
    info!("Found {} nodes in cluster", node_list.items.len());

    // detect platform type
    let platform_type = if let Some(first_node) = node_list.items.first() {
        let empty_labels = BTreeMap::new();
        let labels = first_node.metadata.labels.as_ref().unwrap_or(&empty_labels);
        detect_platform_from_labels(labels).get_platform_type()
    } else {
        PlatformType::GenericKubernetes
    };

    // determine cluster-wide topology strategy
    let cluster_topology_strategy =
        ClusterAnalyzer::determine_cluster_topology_strategy(&node_list.items, &platform_type);

    let detail_level = if config.detailed_labels {
        LabelDetailLevel::Detailed
    } else {
        LabelDetailLevel::Basic
    };

    let mut cluster_report = ClusterReport {
        total_nodes: node_list.items.len(),
        rdma_nodes: 0,
        platform_type,
        api_server_url: kube_config.cluster_url.to_string(),
        topology_detection: None,
        rdma_types: Vec::new(),
        topology_blocks: HashMap::new(),
        topology_gpu_counts: HashMap::new(),
        ib_fabrics: Vec::new(),
        superpods: Vec::new(),
        leafgroups: Vec::new(),
        sriov_networks: Vec::new(),
        nvidia_network_operator_resources: None,
        nodes: Vec::new(),
        gpu_nodes: 0,
        gpu_types: Vec::new(),
        total_gpus: 0,
        image_checked: None,
        cache_check_timestamp: None,
    };

    for node in node_list.items {
        let node_info = ClusterAnalyzer::analyze_node(
            &node,
            detail_level,
            &cluster_topology_strategy,
            config.topology_rule.as_deref(),
        )?;

        if cluster_report.topology_detection.is_none() {
            cluster_report.topology_detection = cluster_topology_strategy.clone();
        }

        if node_info.rdma_capability.is_capable() {
            cluster_report.rdma_nodes += 1;
        }

        // collect GPU statistics
        if let Some(gpu_count) = node_info.gpu_count {
            cluster_report.gpu_nodes += 1;
            cluster_report.total_gpus += gpu_count;
        }

        if let Some(gpu_type) = &node_info.gpu_type
            && !cluster_report.gpu_types.contains(gpu_type)
        {
            cluster_report.gpu_types.push(gpu_type.clone());
        }

        // collect RDMA types
        if let Some(rdma_type) = &node_info.rdma_type
            && !cluster_report.rdma_types.contains(rdma_type)
        {
            cluster_report.rdma_types.push(rdma_type.clone());
        }

        // collect topology blocks
        if let Some(topology_block) = &node_info.topology_block {
            *cluster_report
                .topology_blocks
                .entry(topology_block.clone())
                .or_insert(0) += 1;

            if let Some(gpu_count) = node_info.gpu_count {
                let aggregation_key = if cluster_report.platform_type == PlatformType::CoreWeave {
                    node_info
                        .ib_fabric
                        .clone()
                        .unwrap_or_else(|| topology_block.clone())
                } else {
                    topology_block.clone()
                };

                *cluster_report
                    .topology_gpu_counts
                    .entry(aggregation_key)
                    .or_insert(0) += gpu_count;
            }
        }

        // collect unique values
        if let Some(fabric) = &node_info.ib_fabric
            && !cluster_report.ib_fabrics.contains(fabric)
        {
            cluster_report.ib_fabrics.push(fabric.clone());
        }

        if let Some(superpod) = &node_info.superpod
            && !cluster_report.superpods.contains(superpod)
        {
            cluster_report.superpods.push(superpod.clone());
        }

        if let Some(leafgroup) = &node_info.leafgroup
            && !cluster_report.leafgroups.contains(leafgroup)
        {
            cluster_report.leafgroups.push(leafgroup.clone());
        }

        // apply filter
        match config.rdma_only {
            true if node_info.rdma_capability.is_capable() => {
                cluster_report.nodes.push(node_info);
            }
            true => {} // skip non-RDMA nodes
            false => {
                cluster_report.nodes.push(node_info);
            }
        }
    }

    Ok(cluster_report)
}

/// Pod deployment tracking
#[derive(Debug, Clone)]
struct PodDeployment {
    node_name: String,
    pod_name: String,
}

/// Deploy RoCE detection pods on each RDMA node
async fn deploy_roce_detection_pods(
    client: &Client,
    namespace: &str,
    image: &str,
    rdma_nodes: &[&NodeInfo],
) -> Result<Vec<PodDeployment>> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let mut deployments = Vec::new();

    for node in rdma_nodes {
        let pod_name = format!("roce-detect-{}", sanitize_node_name(&node.name));

        // create pod spec
        let pod = create_roce_detection_pod(&pod_name, &node.name, image);

        // deploy pod
        match pods.create(&PostParams::default(), &pod).await {
            Ok(_) => {
                info!("Deployed RoCE detection pod on node: {}", node.name);
                deployments.push(PodDeployment {
                    node_name: node.name.clone(),
                    pod_name: pod_name.clone(),
                });
            }
            Err(e) => {
                warn!("Failed to deploy pod on node {}: {}", node.name, e);
            }
        }
    }

    Ok(deployments)
}

/// Create a RoCE detection pod specification
fn create_roce_detection_pod(pod_name: &str, node_name: &str, image: &str) -> Pod {
    use k8s_openapi::api::core::v1::{Capabilities, SecurityContext};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            labels: Some(BTreeMap::from([
                ("app".to_string(), "roce-detector".to_string()),
                ("hermes-scan".to_string(), "true".to_string()),
            ])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            host_network: Some(true),
            host_pid: Some(true), // enable access to host PID namespace
            host_ipc: Some(true),
            restart_policy: Some("Never".to_string()),
            node_selector: Some(BTreeMap::from([(
                "kubernetes.io/hostname".to_string(),
                node_name.to_string(),
            )])),
            containers: vec![Container {
                name: "roce-detect".to_string(),
                image: Some(image.to_string()),
                command: Some(vec!["/bin/bash".to_string()]),
                args: Some(vec!["-c".to_string(), create_namespace_detection_script()]),
                security_context: Some(SecurityContext {
                    privileged: Some(true),
                    capabilities: Some(Capabilities {
                        add: Some(vec!["SYS_ADMIN".to_string()]), // required for setns() and nsenter
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                volume_mounts: Some(vec![
                    VolumeMount {
                        name: "dev-infiniband".to_string(),
                        mount_path: "/dev/infiniband".to_string(),
                        ..Default::default()
                    },
                    VolumeMount {
                        name: "host-proc".to_string(),
                        mount_path: "/host/proc".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                    VolumeMount {
                        name: "netns".to_string(),
                        mount_path: "/var/run/netns".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                    VolumeMount {
                        name: "crio".to_string(),
                        mount_path: "/var/run/crio".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }],
            volumes: Some(vec![
                Volume {
                    name: "dev-infiniband".to_string(),
                    host_path: Some(HostPathVolumeSource {
                        path: "/dev/infiniband".to_string(),
                        type_: Some("Directory".to_string()),
                    }),
                    ..Default::default()
                },
                Volume {
                    name: "host-proc".to_string(),
                    host_path: Some(HostPathVolumeSource {
                        path: "/proc".to_string(),
                        type_: Some("Directory".to_string()),
                    }),
                    ..Default::default()
                },
                Volume {
                    name: "netns".to_string(),
                    host_path: Some(HostPathVolumeSource {
                        path: "/var/run/netns".to_string(),
                        type_: Some("DirectoryOrCreate".to_string()),
                    }),
                    ..Default::default()
                },
                Volume {
                    name: "crio".to_string(),
                    host_path: Some(HostPathVolumeSource {
                        path: "/var/run/crio".to_string(),
                        type_: Some("DirectoryOrCreate".to_string()),
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Create the bash script for multi-namespace RoCE detection
fn create_namespace_detection_script() -> String {
    r#"
#!/bin/bash
set -e

# output array to hold all namespace configs
echo '{'
echo '  "namespaces": ['

first_ns=true

# 1. detect host namespace RoCE config
echo "  {"
echo "    \"namespace_type\": \"Host\","
echo "    \"namespace_id\": \"host\","
echo -n "    \"config\": "
/usr/local/bin/roce-detector --format json 2>/dev/null || echo '{"active_hcas":[],"nccl_hcas":[],"ucx_hcas":[],"gid_index_counts":{},"hca_details":[]}'
echo "  }"
first_ns=false

# 2. find all container PIDs and detect in their namespaces
# check if crictl is available
if command -v crictl &> /dev/null; then
  # get all running containers on this node
  for container_id in $(crictl ps -q 2>/dev/null || true); do
    # get container info
    container_info=$(crictl inspect "$container_id" 2>/dev/null || continue)

    # extract PID and pod info
    pid=$(echo "$container_info" | grep -o '"pid": [0-9]*' | head -1 | awk '{print $2}')
    pod_name=$(echo "$container_info" | grep -o '"io.kubernetes.pod.name": "[^"]*"' | cut -d'"' -f4)
    pod_namespace=$(echo "$container_info" | grep -o '"io.kubernetes.pod.namespace": "[^"]*"' | cut -d'"' -f4)

    if [ -z "$pid" ] || [ -z "$pod_name" ]; then
      continue
    fi

    # check if this container has RDMA devices (VFs) in its namespace
    # try to enter the network namespace and check for RDMA devices
    if nsenter -t "$pid" -n test -d /dev/infiniband 2>/dev/null; then
      # output separator
      if [ "$first_ns" = false ]; then
        echo ","
      fi
      first_ns=false

      # output pod namespace config
      echo "  {"
      echo "    \"namespace_type\": \"Pod\","
      echo "    \"namespace_id\": \"$pod_name\","
      echo "    \"pod_name\": \"$pod_name\","
      echo "    \"pod_namespace\": \"$pod_namespace\","
      echo "    \"pid\": $pid,"
      echo -n "    \"config\": "

      # run roce-detector in this container's network namespace
      nsenter -t "$pid" -n /usr/local/bin/roce-detector --format json 2>/dev/null || echo '{"active_hcas":[],"nccl_hcas":[],"ucx_hcas":[],"gid_index_counts":{},"hca_details":[]}'

      echo "  }"
    fi
  done
fi

# close the JSON array and object
echo '  ]'
echo '}'
"#.to_string()
}

/// Sanitize node name for use in pod name (DNS-1123 compliant)
fn sanitize_node_name(node_name: &str) -> String {
    node_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40) // keep pod name under 63 chars total
        .collect()
}

/// Result of pod completion
#[derive(Debug)]
struct PodResult {
    node_name: String,
    pod_name: String,
    succeeded: bool,
}

/// Wait for all pods to complete (succeed or fail)
async fn wait_for_pods_completion(
    client: &Client,
    namespace: &str,
    deployments: &[PodDeployment],
    timeout: Duration,
) -> Result<Vec<PodResult>> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let start = std::time::Instant::now();

    let mut results = Vec::new();
    let mut pending_pods: Vec<_> = deployments.iter().collect();

    while !pending_pods.is_empty() && start.elapsed() < timeout {
        let mut still_pending = Vec::new();

        for deployment in &pending_pods {
            match pods.get(&deployment.pod_name).await {
                Ok(pod) => {
                    let status = pod.status.as_ref();
                    let phase = status.and_then(|s| s.phase.as_deref());

                    match phase {
                        Some("Succeeded") => {
                            info!("Pod {} completed successfully", deployment.pod_name);
                            results.push(PodResult {
                                node_name: deployment.node_name.clone(),
                                pod_name: deployment.pod_name.clone(),
                                succeeded: true,
                            });
                        }
                        Some("Failed") => {
                            warn!("Pod {} failed", deployment.pod_name);
                            results.push(PodResult {
                                node_name: deployment.node_name.clone(),
                                pod_name: deployment.pod_name.clone(),
                                succeeded: false,
                            });
                        }
                        _ => {
                            // still running or pending
                            still_pending.push(*deployment);
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to get status for pod {}: {}",
                        deployment.pod_name, e
                    );
                    still_pending.push(*deployment);
                }
            }
        }

        pending_pods = still_pending;

        if !pending_pods.is_empty() {
            sleep(Duration::from_secs(2)).await;
        }
    }

    // mark timed-out pods as failed
    for deployment in pending_pods {
        warn!("Pod {} timed out", deployment.pod_name);
        results.push(PodResult {
            node_name: deployment.node_name.clone(),
            pod_name: deployment.pod_name.clone(),
            succeeded: false,
        });
    }

    Ok(results)
}

/// Collect RoCE configurations from pod logs
async fn collect_roce_configs(
    client: &Client,
    namespace: &str,
    pod_results: &[PodResult],
) -> Result<HashMap<String, RoceConfig>> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let mut configs = HashMap::new();

    for result in pod_results {
        if !result.succeeded {
            warn!(
                "Skipping RoCE config collection for failed pod: {}",
                result.pod_name
            );
            continue;
        }

        // get pod logs
        match pods.logs(&result.pod_name, &Default::default()).await {
            Ok(logs) => {
                // parse JSON from logs (may contain ANSI codes, so we need to strip them)
                match parse_roce_config_from_logs(&logs) {
                    Ok(config) => {
                        info!(
                            "Parsed RoCE config for node {}: {} active HCAs",
                            result.node_name,
                            config.active_hcas().len()
                        );
                        configs.insert(result.node_name.clone(), config);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse RoCE config from pod {} logs: {}",
                            result.pod_name, e
                        );
                    }
                }
            }
            Err(e) => {
                warn!("Failed to get logs from pod {}: {}", result.pod_name, e);
            }
        }
    }

    Ok(configs)
}

/// Parse RoceConfig from pod logs (handles ANSI codes and multi-namespace output)
fn parse_roce_config_from_logs(logs: &str) -> Result<RoceConfig> {
    // strip ANSI escape codes
    let cleaned = strip_ansi_escapes::strip_str(logs);

    // find the start of JSON (first line starting with '{')
    // then collect all remaining lines as the JSON block
    let lines: Vec<&str> = cleaned.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            // found start of JSON, join this line and all remaining lines
            let json_block = lines[idx..].join("\n");

            // try to parse as multi-namespace output first
            if let Ok(multi_ns) = serde_json::from_str::<MultiNamespaceOutput>(&json_block) {
                debug!(
                    "Parsed multi-namespace output with {} namespaces",
                    multi_ns.namespaces.len()
                );
                return aggregate_namespace_configs(multi_ns);
            }

            // fallback to single-namespace (old format)
            match serde_json::from_str::<RoceConfig>(&json_block) {
                Ok(config) => return Ok(config),
                Err(e) => {
                    // if parsing fails, try next occurrence of '{'
                    debug!("Failed to parse JSON block starting at line {}: {}", idx, e);
                    continue;
                }
            }
        }
    }

    // fallback: try to parse entire output as single-namespace JSON
    serde_json::from_str::<RoceConfig>(&cleaned)
        .context("Failed to parse RoceConfig from logs - no valid JSON block found")
}

/// Aggregate multi-namespace detection results into a single RoceConfig
fn aggregate_namespace_configs(multi_ns: MultiNamespaceOutput) -> Result<RoceConfig> {
    let mut all_active_hcas = Vec::new();
    let mut all_nccl_hcas = Vec::new();
    let mut all_ucx_hcas = Vec::new();
    let mut all_gid_index_counts = HashMap::new();
    let mut all_hca_details = Vec::new();
    let mut namespace_configs = Vec::new();

    // convert namespace type string to enum
    let parse_namespace_type = |s: &str| -> NamespaceType {
        match s {
            "Host" => NamespaceType::Host,
            "Pod" => NamespaceType::Pod,
            "NetworkNamespace" => NamespaceType::NetworkNamespace,
            _ => NamespaceType::NetworkNamespace, // default
        }
    };

    for ns_result in multi_ns.namespaces {
        let config = ns_result.config;

        // aggregate HCAs (avoid duplicates)
        for hca in &config.active_hcas {
            if !all_active_hcas.contains(hca) {
                all_active_hcas.push(hca.clone());
            }
        }
        for hca in &config.nccl_hcas {
            if !all_nccl_hcas.contains(hca) {
                all_nccl_hcas.push(hca.clone());
            }
        }
        for hca in &config.ucx_hcas {
            if !all_ucx_hcas.contains(hca) {
                all_ucx_hcas.push(hca.clone());
            }
        }

        // aggregate GID index counts
        for (idx, count) in config.gid_index_counts.iter() {
            *all_gid_index_counts.entry(*idx).or_insert(0) += count;
        }

        // aggregate HCA details (avoid duplicates by name)
        for detail in &config.hca_details {
            if !all_hca_details
                .iter()
                .any(|d: &crate::models::HcaDetail| d.name == detail.name)
            {
                all_hca_details.push(detail.clone());
            }
        }

        // create namespace-specific config
        namespace_configs.push(NamespaceRoceConfig {
            namespace_type: parse_namespace_type(&ns_result.namespace_type),
            namespace_id: ns_result.namespace_id,
            pod_name: ns_result.pod_name,
            pod_namespace: ns_result.pod_namespace,
            pid: ns_result.pid,
            active_hcas: config.active_hcas,
            gid_index: config.gid_index,
            gid_index_counts: config.gid_index_counts,
            hca_details: config.hca_details,
        });
    }

    // determine global GID index (if all namespaces agree)
    let unique_gid_indices: Vec<u32> = namespace_configs
        .iter()
        .filter_map(|c| c.gid_index)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let global_gid_index = if unique_gid_indices.len() == 1 {
        Some(unique_gid_indices[0])
    } else {
        None
    };

    // detect GID mismatches
    let gid_mismatch_detected = unique_gid_indices.len() > 1;

    // identify affected pods
    let affected_pods = if gid_mismatch_detected {
        Some(
            namespace_configs
                .iter()
                .filter(|c| c.namespace_type == NamespaceType::Pod)
                .filter_map(|c| c.pod_name.clone())
                .collect(),
        )
    } else {
        None
    };

    Ok(RoceConfig {
        active_hcas: all_active_hcas,
        nccl_hcas: all_nccl_hcas,
        ucx_hcas: all_ucx_hcas,
        gid_index: global_gid_index,
        gid_index_counts: all_gid_index_counts,
        hca_details: all_hca_details,
        namespace_configs: Some(namespace_configs),
        gid_mismatch_detected: Some(gid_mismatch_detected),
        affected_pods,
    })
}

/// Merge RoCE configurations into cluster report
fn merge_roce_configs_into_report(
    cluster_report: &mut ClusterReport,
    roce_configs: &HashMap<String, RoceConfig>,
) {
    for node in &mut cluster_report.nodes {
        if let Some(config) = roce_configs.get(&node.name) {
            node.roce_config = Some(config.clone());
        }
    }
}

/// Cleanup RoCE detection pods
async fn cleanup_pods(
    client: &Client,
    namespace: &str,
    deployments: &[PodDeployment],
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    for deployment in deployments {
        match pods
            .delete(&deployment.pod_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {
                info!("Deleted pod: {}", deployment.pod_name);
            }
            Err(e) => {
                error!("Failed to delete pod {}: {}", deployment.pod_name, e);
            }
        }
    }

    Ok(())
}

/// Output cluster report with RoCE configs
fn output_report(cluster_report: &ClusterReport, config: &RoceScanConfig) -> Result<()> {
    // save to file if requested
    if let Some(save_path) = &config.save_to {
        info!("Saving scan results to: {}", save_path);
        let json_data = serde_json::to_string_pretty(&cluster_report)?;
        std::fs::write(save_path, json_data)?;
        info!("Scan results saved to: {}", save_path);
    }

    // format and print
    let formatter = get_formatter(&config.format);
    let output = formatter.format_report(cluster_report)?;

    if config.format == "table" {
        print!("{}", output);
    } else {
        println!("{}", output);
    }

    Ok(())
}
