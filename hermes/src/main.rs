use anyhow::{Result, bail};
use clap::Parser;
use k8s_openapi::api::core::v1::Node;
use kube::{Api, Client, Config, api::ListParams};
use std::collections::{BTreeMap, HashMap};
use tracing::{info, warn};

use hermes::analyzer::ClusterAnalyzer;
use hermes::cache::CacheManager;
use hermes::crds::sriovnetworks::SriovNetwork;
use hermes::formatters::*;
use hermes::models::*;
use hermes::platforms::*;
use hermes::roce_scan;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Parser)]
enum Commands {
    /// Scan cluster for topology and RDMA capabilities (default command)
    Scan {
        /// Output format (json, yaml, markdown, table)
        #[arg(short, long, default_value = "table")]
        format: String,

        /// Show only nodes with RDMA capabilities
        #[arg(long)]
        rdma_only: bool,

        /// Include detailed platform-specific labels and networking info
        #[arg(long)]
        detailed_labels: bool,

        /// Show resource usage (CPU/memory/GPU) by querying running pods
        #[arg(long)]
        show_usage: bool,

        /// Save scan results to file for use by self-test
        #[arg(long)]
        save_to: Option<String>,

        /// Skip cache and force fresh scan
        #[arg(long)]
        no_cache: bool,

        /// Cache TTL in hours (default: 24)
        #[arg(long)]
        cache_ttl: Option<i64>,

        /// Custom rule to extract topology from nodes
        /// Supports two formats:
        ///   - regex:PATTERN - Regex with optional capture group (e.g., 'regex:r(\d+)')
        ///   - CEL expression - Uses node_name and node_labels variables (e.g., 'node_name')
        #[arg(long)]
        topology_rule: Option<String>,
    },
    /// Run self-test workload to validate RDMA connectivity
    SelfTest {
        /// Namespace to deploy test workload
        #[arg(short, long, default_value = "default")]
        namespace: String,

        /// Read workload manifest from stdin instead of embedded defaults
        #[arg(long)]
        from_stdin: bool,

        /// Keep test resources after completion (for debugging)
        #[arg(long)]
        no_cleanup: bool,

        /// Load cluster scan data from file (instead of re-scanning)
        #[arg(long)]
        load_from: Option<String>,

        /// Dry run mode: render manifests to stdout without deploying
        #[arg(long)]
        dry_run: bool,

        /// SR-IOV network name for RoCE (e.g., "roce-p2" or "namespace/roce-p2")
        #[arg(long)]
        sriov_network: Option<String>,

        /// Use SR-IOV VF resources for workloads (default: true). Use --no-prefer-sriov-resources for direct PF access.
        /// When enabled: requests network-specific SR-IOV resources (e.g., openshift.io/p2rdma) - good for multi-tenancy
        /// When disabled: requests generic RDMA resources (e.g., rdma/roce_gdr) for direct physical NIC access - maximum performance
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        prefer_sriov_resources: bool,

        /// Request GPU resources in test pods (enables GDRCOPY and CUDA IPC transports)
        #[arg(long, default_value = "true")]
        request_gpu: bool,

        /// Skip cleanup on CTRL-C (default: cleanup on interrupt)
        #[arg(long)]
        no_cleanup_on_signal: bool,

        /// Workload name to run (default: nixl-transfer-test)
        #[arg(short, long)]
        workload: Option<String>,

        /// Container image to use for test workloads
        #[arg(
            long,
            default_value = "ghcr.io/llm-d/llm-d-cuda-dev:sha-d58731d@sha256:ba067a81b28546650a5496c3093a21b249c3f0c60d0d305ddcd1907e632e6edd"
        )]
        image: String,

        /// Override number of GPUs per node (default: use workload's requirement)
        #[arg(long)]
        gpus_per_node: Option<u32>,

        /// Custom CEL rule to extract topology from nodes
        /// Example: 'string(int(extract(node_name, "r(\\d+)")) / 10)'
        #[arg(long)]
        topology_rule: Option<String>,

        /// UCX GID index for RoCE (default: auto-detect via UCX)
        /// Override only if you need a specific GID index (e.g., "3")
        #[arg(long)]
        ucx_gid_index: Option<String>,
    },
    /// Scan cluster and detect RoCE configuration on each RDMA node
    ScanRoce {
        /// Output format (json, yaml, markdown, table)
        #[arg(short, long, default_value = "table")]
        format: String,

        /// Only scan nodes with RDMA capabilities
        #[arg(long)]
        rdma_only: bool,

        /// Container image for RoCE detection pods
        #[arg(long, default_value = "quay.io/wseaton/roce-detector:latest")]
        image: String,

        /// Namespace to deploy detection pods
        #[arg(short, long, default_value = "default")]
        namespace: String,

        /// Clean up detection pods after completion
        #[arg(long, default_value_t = true)]
        cleanup: bool,

        /// Custom topology rule
        #[arg(long)]
        topology_rule: Option<String>,

        /// Include detailed platform-specific labels and networking info
        #[arg(long)]
        detailed_labels: bool,

        /// Save scan results to file
        #[arg(long)]
        save_to: Option<String>,
    },
    /// Select optimal node set for RDMA workloads
    SelectNodes {
        /// Number of nodes to select
        #[arg(long)]
        num_nodes: Option<usize>,

        /// GPUs per node (local ranks)
        #[arg(long)]
        gpus_per_node: Option<u32>,

        /// Total GPUs needed (alternative to num-nodes)
        #[arg(long)]
        total_gpus: Option<u32>,

        /// Minimum GPUs per node
        #[arg(long)]
        min_gpus_per_node: Option<u32>,

        /// Only select InfiniBand nodes
        #[arg(long)]
        ib_only: bool,

        /// Output format (json, shell, helm-values)
        #[arg(short, long, default_value = "json")]
        format: String,

        /// Prefer nodes in same topology block (default: true)
        #[arg(long, default_value_t = true)]
        prefer_same_block: bool,

        /// Custom topology rule
        #[arg(long)]
        topology_rule: Option<String>,
    },
    /// Helm plugin integration (internal use by helm plugin)
    #[command(hide = true)]
    HelmPlugin {
        /// Raw arguments passed from helm plugin
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

struct ScanOptions {
    format: String,
    node_filter: NodeFilter,
    detail_level: LabelDetailLevel,
    show_usage: bool,
    save_to: Option<String>,
    cache_mode: CacheMode,
    cache_ttl: Option<i64>,
    topology_rule: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();

    match args.command {
        Commands::Scan {
            format,
            rdma_only,
            detailed_labels,
            show_usage,
            save_to,
            no_cache,
            cache_ttl,
            topology_rule,
        } => {
            let node_filter = if rdma_only {
                NodeFilter::RdmaOnly
            } else {
                NodeFilter::All
            };
            let detail_level = if detailed_labels {
                LabelDetailLevel::Detailed
            } else {
                LabelDetailLevel::Basic
            };
            let cache_mode = if no_cache {
                CacheMode::SkipCache
            } else {
                CacheMode::UseCache
            };

            let options = ScanOptions {
                format,
                node_filter,
                detail_level,
                show_usage,
                save_to,
                cache_mode,
                cache_ttl,
                topology_rule,
            };

            run_scan(options).await
        }
        Commands::SelfTest {
            namespace,
            from_stdin,
            no_cleanup,
            load_from,
            dry_run,
            sriov_network,
            prefer_sriov_resources,
            request_gpu,
            no_cleanup_on_signal,
            workload,
            image,
            gpus_per_node,
            topology_rule,
            ucx_gid_index,
        } => {
            use hermes::self_test::SelfTestConfig;
            use std::time::Duration;

            let config = SelfTestConfig {
                namespace,
                workload_source: if from_stdin {
                    WorkloadSource::Stdin
                } else {
                    WorkloadSource::Embedded
                },
                cleanup_mode: if no_cleanup {
                    CleanupMode::NoCleanup
                } else {
                    CleanupMode::Cleanup
                },
                execution_mode: if dry_run {
                    ExecutionMode::DryRun
                } else {
                    ExecutionMode::Execute
                },
                timeout: Duration::from_secs(300),
                sriov_network,
                prefer_sriov_resources,
                gpu_requirement: if request_gpu {
                    GpuRequirement::Required
                } else {
                    GpuRequirement::NotRequired
                },
                signal_handling: if no_cleanup_on_signal {
                    SignalHandling::NoCleanupOnSignal
                } else {
                    SignalHandling::CleanupOnSignal
                },
                workload,
                image,
                load_from,
                gpus_per_node,
                cache_check: ImageCacheCheck::CheckCache,
                cache_ttl_seconds: 1800, // 30 minutes
                cache_check_timeout: Duration::from_secs(5),
                topology_rule,
                ucx_gid_index,
            };

            run_self_test(config).await
        }
        Commands::ScanRoce {
            format,
            rdma_only,
            image,
            namespace,
            cleanup,
            topology_rule,
            detailed_labels,
            save_to,
        } => {
            let config = roce_scan::RoceScanConfig {
                format,
                rdma_only,
                image,
                namespace,
                cleanup,
                topology_rule,
                detailed_labels,
                save_to,
            };

            roce_scan::run_roce_scan(config).await
        }
        Commands::SelectNodes {
            num_nodes,
            gpus_per_node,
            total_gpus,
            min_gpus_per_node,
            ib_only,
            format,
            prefer_same_block,
            topology_rule,
        } => {
            use hermes::node_selector::NodeSelectionParams;

            let params = NodeSelectionParams {
                num_nodes,
                gpus_per_node,
                total_gpus,
                min_gpus_per_node,
                ib_only,
                prefer_same_block,
            };

            run_select_nodes(params, format, topology_rule).await
        }
        Commands::HelmPlugin { args } => run_helm_plugin(args).await,
    }
}

async fn run_scan(options: ScanOptions) -> Result<()> {
    info!("Starting cluster scan...");

    // handle proxy settings for client configuration
    let (client, config) = if let Ok(proxy) = std::env::var("HTTPS_PROXY") {
        info!("Using proxy: {}", proxy);
        let mut config = Config::infer().await?;

        // handle certificate validation issues common with corporate proxies
        // check if we should accept invalid certificates
        if std::env::var("KUBE_INSECURE_TLS").is_ok()
            || std::env::var("KUBERNETES_INSECURE_TLS").is_ok()
        {
            info!("Disabling TLS certificate verification due to environment variable");
            config.accept_invalid_certs = true;
        }

        // note: kube-rs doesn't directly support proxy, so we rely on system settings
        // the HTTPS_PROXY env var should be picked up by the underlying HTTP client
        (Client::try_from(config.clone())?, config)
    } else {
        let config = Config::infer().await?;
        (Client::try_from(config.clone())?, config)
    };

    // check cache unless cache mode is SkipCache
    let cache_manager = CacheManager::new()?;
    let context_key = CacheManager::generate_context_key(&config);

    let cluster_report = if options.cache_mode.should_use_cache()
        && let Some(cached_report) = cache_manager.load(&context_key, options.cache_ttl)?
    {
        // use cached report
        let mut cluster_report = cached_report;

        // apply RDMA filter if requested
        if matches!(options.node_filter, NodeFilter::RdmaOnly) {
            cluster_report
                .nodes
                .retain(|node| node.rdma_capability.is_capable());
        }

        cluster_report
    } else {
        let nodes: Api<Node> = Api::all(client.clone());

        let node_list = nodes.list(&ListParams::default()).await?;
        info!("Found {} nodes in cluster", node_list.items.len());

        // detect platform type from first node
        let platform_type = if let Some(first_node) = node_list.items.first() {
            let empty_labels = BTreeMap::new();
            let labels = first_node.metadata.labels.as_ref().unwrap_or(&empty_labels);
            detect_platform_from_labels(labels).get_platform_type()
        } else {
            PlatformType::GenericKubernetes
        };

        // determine cluster-wide topology strategy before analyzing individual nodes
        let cluster_topology_strategy =
            ClusterAnalyzer::determine_cluster_topology_strategy(&node_list.items, &platform_type);

        // detect NVIDIA Network Operator and extract RDMA resource names
        // TODO: nvidia_network module removed, need to re-implement detection
        let nvidia_rdma_resources: Option<Vec<String>> = None;

        let mut cluster_report = ClusterReport {
            total_nodes: node_list.items.len(),
            rdma_nodes: 0,
            platform_type,
            api_server_url: config.cluster_url.to_string(),
            topology_detection: None,
            rdma_types: Vec::new(),
            topology_blocks: HashMap::new(),
            topology_gpu_counts: HashMap::new(),
            ib_fabrics: Vec::new(),
            superpods: Vec::new(),
            leafgroups: Vec::new(),
            sriov_networks: Vec::new(),
            nvidia_network_operator_resources: nvidia_rdma_resources.clone(),
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
                options.detail_level,
                &cluster_topology_strategy,
                options.topology_rule.as_deref(),
            )?;

            // set cluster topology detection from strategy
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

            // collect unique RDMA types
            if let Some(rdma_type) = &node_info.rdma_type
                && !cluster_report.rdma_types.contains(rdma_type)
            {
                cluster_report.rdma_types.push(rdma_type.clone());
            }

            // collect topology blocks and GPU counts per block
            if let Some(topology_block) = &node_info.topology_block {
                *cluster_report
                    .topology_blocks
                    .entry(topology_block.clone())
                    .or_insert(0) += 1;

                if let Some(gpu_count) = node_info.gpu_count {
                    // for CoreWeave, aggregate GPUs by fabric instead of leafgroup
                    let aggregation_key = if cluster_report.platform_type == PlatformType::CoreWeave
                    {
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

            // collect unique values for summary
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

            match options.node_filter {
                NodeFilter::RdmaOnly if node_info.rdma_capability.is_capable() => {
                    cluster_report.nodes.push(node_info);
                }
                NodeFilter::All => {
                    cluster_report.nodes.push(node_info);
                }
                _ => {}
            }
        }

        // log aggregated topology rule evaluation failures
        let topology_rule_failures: Vec<_> = cluster_report
            .nodes
            .iter()
            .filter(|n| n.topology_rule_error.is_some())
            .collect();
        if !topology_rule_failures.is_empty() {
            warn!(
                "Failed to evaluate topology rule for {} node(s)",
                topology_rule_failures.len()
            );
        }

        // detect SR-IOV networks for OpenShift (not applicable to CoreWeave or GKE)
        if matches!(
            platform_type,
            PlatformType::OpenShift | PlatformType::GenericKubernetes
        ) {
            info!("Detecting SR-IOV networks across all namespaces...");
            cluster_report.sriov_networks = detect_sriov_networks(&client).await;
        }

        // populate resource allocations if requested
        if options.show_usage {
            info!("Querying pod allocations for resource usage...");
            use k8s_openapi::api::core::v1::Pod;
            let pods: Api<Pod> = Api::all(client.clone());
            let pod_list = pods.list(&ListParams::default()).await?;

            ClusterAnalyzer::populate_gpu_allocations(&mut cluster_report.nodes, &pod_list.items);
            ClusterAnalyzer::populate_resource_allocations(
                &mut cluster_report.nodes,
                &pod_list.items,
            );
        }

        // save scan results to cache (unless cache mode is SkipCache)
        if options.cache_mode.should_use_cache() {
            cache_manager.save(&context_key, &cluster_report)?;
        }

        cluster_report
    };

    // save scan results to file if requested
    if let Some(save_path) = &options.save_to {
        info!("Saving scan results to: {}", save_path);
        let json_data = serde_json::to_string_pretty(&cluster_report)?;
        std::fs::write(save_path, json_data)?;
        info!("Scan results saved to: {}", save_path);
    }

    let formatter = get_formatter(&options.format);
    let output = formatter.format_report(&cluster_report)?;

    // only use println! for table format since it's already formatted
    // for json/yaml, the output already includes formatting
    if options.format == "table" {
        print!("{}", output);
    } else {
        println!("{}", output);
    }

    Ok(())
}

/// detect SR-IOV networks from the OpenShift operator namespace
async fn detect_sriov_networks(client: &Client) -> Vec<SriovNetworkInfo> {
    // openshift SR-IOV networks are defined in the operator namespace
    let operator_namespace = "openshift-sriov-network-operator";
    let sriov_api: Api<SriovNetwork> = Api::namespaced(client.clone(), operator_namespace);

    match sriov_api.list(&ListParams::default()).await {
        Ok(network_list) => {
            info!(
                "Found {} SR-IOV networks in {}",
                network_list.items.len(),
                operator_namespace
            );
            network_list
                .items
                .iter()
                .filter_map(|net| {
                    let name = net.metadata.name.clone()?;
                    let namespace = net
                        .metadata
                        .namespace
                        .clone()
                        .unwrap_or_else(|| operator_namespace.to_string());
                    // get the target namespace where the network will be available
                    let target_namespace = net
                        .spec
                        .network_namespace
                        .clone()
                        .unwrap_or_else(|| namespace.clone());

                    Some(SriovNetworkInfo {
                        name,
                        namespace: target_namespace,
                        resource_name: net.spec.resource_name.clone(),
                        vlan: net.spec.vlan,
                    })
                })
                .collect()
        }
        Err(e) => {
            // sr-iov might not be available on this cluster
            info!("SR-IOV networks not available on this cluster: {}", e);
            Vec::new()
        }
    }
}

async fn run_self_test(config: hermes::self_test::SelfTestConfig) -> Result<()> {
    use hermes::self_test::SelfTestOrchestrator;

    info!("Starting self-test in namespace: {}", config.namespace);
    info!(
        "Config: execution_mode={:?}, gpu_requirement={:?}, workload={:?}, image={}",
        config.execution_mode, config.gpu_requirement, config.workload, config.image
    );

    // check if we should load scan data (for future use in node selection optimization)
    let _cached_scan = if let Some(scan_file) = &config.load_from {
        // load from explicit file path
        info!("Loading scan data from file: {}", scan_file);
        let scan_data = std::fs::read_to_string(scan_file)?;
        let report: ClusterReport = serde_json::from_str(&scan_data)?;
        info!("Loaded scan data from: {}", scan_file);
        Some(report)
    } else {
        // try to load from cache
        let cache_manager = CacheManager::new()?;
        let config = Config::infer().await?;
        let context_key = CacheManager::generate_context_key(&config);

        if let Some(report) = cache_manager.load(&context_key, None)? {
            info!("Using cached cluster scan data");
            Some(report)
        } else {
            info!("Will perform fresh cluster scan for node selection");
            None
        }
    };

    // note: cached_scan can be used in future for intelligent node selection optimization

    if config.execution_mode.is_dry_run() {
        info!("Dry run mode: will render manifests to stdout without deploying");
    }

    // setup kubernetes client (unless in dry run mode)
    let client = if config.execution_mode.is_dry_run() {
        // create a dummy client for dry run mode
        kube::Client::try_default().await.unwrap_or_else(|_| {
            // if we can't connect to cluster in dry run, that's ok
            panic!("Dry run mode requires cluster access for node selection")
        })
    } else {
        // handle proxy settings for client configuration (same as scan)
        if let Ok(proxy) = std::env::var("HTTPS_PROXY") {
            info!("Using proxy: {}", proxy);
            let mut kube_config = kube::Config::infer().await?;

            if std::env::var("KUBE_INSECURE_TLS").is_ok()
                || std::env::var("KUBERNETES_INSECURE_TLS").is_ok()
            {
                info!("Disabling TLS certificate verification due to environment variable");
                kube_config.accept_invalid_certs = true;
            }

            kube::Client::try_from(kube_config)?
        } else {
            kube::Client::try_default().await?
        }
    };

    let orchestrator = SelfTestOrchestrator::new(client, config);
    let _result = orchestrator.run().await?;

    Ok(())
}

async fn run_select_nodes(
    params: hermes::node_selector::NodeSelectionParams,
    output_format: String,
    topology_rule: Option<String>,
) -> Result<()> {
    use hermes::node_selector::select_nodes_from_report;

    info!("Scanning cluster for node selection...");

    // scan cluster using existing infrastructure
    let scan_options = ScanOptions {
        format: "json".to_string(), // internal format
        node_filter: NodeFilter::All,
        detail_level: LabelDetailLevel::Basic,
        show_usage: false,
        save_to: None,
        cache_mode: CacheMode::UseCache,
        cache_ttl: None,
        topology_rule,
    };

    // reuse run_scan logic to get cluster report
    let (client, config) = if let Ok(proxy) = std::env::var("HTTPS_PROXY") {
        info!("Using proxy: {}", proxy);
        let mut config = Config::infer().await?;

        if std::env::var("KUBE_INSECURE_TLS").is_ok()
            || std::env::var("KUBERNETES_INSECURE_TLS").is_ok()
        {
            info!("Disabling TLS certificate verification due to environment variable");
            config.accept_invalid_certs = true;
        }

        (Client::try_from(config.clone())?, config)
    } else {
        let config = Config::infer().await?;
        (Client::try_from(config.clone())?, config)
    };

    // check cache
    let cache_manager = CacheManager::new()?;
    let context_key = CacheManager::generate_context_key(&config);

    let cluster_report = if scan_options.cache_mode.should_use_cache()
        && let Some(cached_report) = cache_manager.load(&context_key, scan_options.cache_ttl)?
    {
        info!("Using cached cluster scan data");
        cached_report
    } else {
        info!("Performing fresh cluster scan...");
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

        let mut cluster_report = ClusterReport {
            total_nodes: node_list.items.len(),
            rdma_nodes: 0,
            platform_type,
            api_server_url: config.cluster_url.to_string(),
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
                scan_options.detail_level,
                &cluster_topology_strategy,
                scan_options.topology_rule.as_deref(),
            )?;

            if cluster_report.topology_detection.is_none() {
                cluster_report.topology_detection = cluster_topology_strategy.clone();
            }

            if node_info.rdma_capability.is_capable() {
                cluster_report.rdma_nodes += 1;
            }

            cluster_report.nodes.push(node_info);
        }

        // save to cache
        if scan_options.cache_mode.should_use_cache() {
            cache_manager.save(&context_key, &cluster_report)?;
        }

        cluster_report
    };

    info!(
        "Found {} RDMA-capable nodes on {} platform",
        cluster_report.rdma_nodes, cluster_report.platform_type
    );

    // select nodes using topology-aware logic
    let selection = select_nodes_from_report(&cluster_report, &params)?;

    // output in requested format
    let output = match output_format.as_str() {
        "json" => selection.to_json()?,
        "shell" => selection.to_shell(),
        "helm-values" => selection.to_helm_values()?,
        _ => bail!("Unknown output format: {}", output_format),
    };

    println!("{}", output);

    Ok(())
}

async fn run_helm_plugin(raw_args: Vec<String>) -> Result<()> {
    use hermes::node_selector::NodeSelectionParams;
    use std::process::Command;

    // detect debug mode from helm
    let is_debug = std::env::var("HELM_DEBUG")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    if is_debug {
        eprintln!("DEBUG: Helm plugin invoked with args: {:?}", raw_args);
        eprintln!(
            "DEBUG: HELM_NAMESPACE: {}",
            std::env::var("HELM_NAMESPACE").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: HELM_KUBECONTEXT: {}",
            std::env::var("HELM_KUBECONTEXT").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: HELM_KUBEAPISERVER: {}",
            std::env::var("HELM_KUBEAPISERVER").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: HELM_KUBEINSECURE_SKIP_TLS_VERIFY: {}",
            std::env::var("HELM_KUBEINSECURE_SKIP_TLS_VERIFY").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: HELM_BIN: {}",
            std::env::var("HELM_BIN").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: HELM_PLUGIN_DIR: {}",
            std::env::var("HELM_PLUGIN_DIR").unwrap_or_default()
        );
        eprintln!(
            "DEBUG: KUBECONFIG: {}",
            std::env::var("KUBECONFIG").unwrap_or_default()
        );
    }

    // handle help flags
    if raw_args.iter().any(|arg| arg == "-h" || arg == "--help") {
        eprintln!("Hermes Helm Plugin - Inject cluster topology into Helm values");
        eprintln!();
        eprintln!("USAGE:");
        eprintln!(
            "    helm hermes <install|upgrade|template> <release> <chart> [HERMES_FLAGS] [HELM_FLAGS]"
        );
        eprintln!();
        eprintln!("HERMES FLAGS:");
        eprintln!("    --num-nodes <N>           Select N nodes for the workload");
        eprintln!("    --gpus-per-node <N>       Select nodes with N GPUs each");
        eprintln!("    --total-gpus <N>          Select nodes with total N GPUs across all nodes");
        eprintln!("    --min-gpus-per-node <N>   Select nodes with at least N GPUs each");
        eprintln!("    --ib-only                 Only select InfiniBand-capable nodes");
        eprintln!(
            "    --prefer-same-block       Prefer nodes in the same topology block (default: true)"
        );
        eprintln!("    --topology-rule <RULE>    Custom topology extraction rule (regex or CEL)");
        eprintln!();
        eprintln!("EXAMPLES:");
        eprintln!("    # Install release with 2 IB nodes from same topology block");
        eprintln!("    helm hermes install my-release ./chart --num-nodes 2 --ib-only");
        eprintln!();
        eprintln!("    # Upgrade with 64 total GPUs distributed across nodes");
        eprintln!("    helm hermes upgrade my-release ./chart --total-gpus 64");
        eprintln!();
        eprintln!("    # Template with 8 GPUs per node, 4 nodes minimum");
        eprintln!("    helm hermes template my-release ./chart --gpus-per-node 8 --num-nodes 4");
        return Ok(());
    }

    // parse arguments to separate helm command, hermes flags, and helm flags
    let mut helm_cmd: Option<String> = None;
    let mut release_name: Option<String> = None;
    let mut chart: Option<String> = None;
    let mut hermes_args = Vec::new();
    let mut helm_args = Vec::new();

    let mut iter = raw_args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "install" | "upgrade" | "template" => {
                helm_cmd = Some(arg.clone());
                // next two args should be release name and chart
                release_name = iter.next().cloned();
                chart = iter.next().cloned();
            }
            "--num-nodes"
            | "--gpus-per-node"
            | "--total-gpus"
            | "--min-gpus-per-node"
            | "--topology-rule" => {
                hermes_args.push(arg.clone());
                if let Some(value) = iter.next() {
                    hermes_args.push(value.clone());
                }
            }
            "--ib-only" | "--prefer-same-block" => {
                hermes_args.push(arg.clone());
            }
            _ => {
                // pass through to helm
                helm_args.push(arg.clone());
            }
        }
    }

    let helm_cmd = helm_cmd.ok_or_else(|| anyhow::anyhow!("No helm command specified"))?;
    let release_name = release_name.ok_or_else(|| anyhow::anyhow!("No release name specified"))?;
    let chart = chart.ok_or_else(|| anyhow::anyhow!("No chart specified"))?;

    // parse hermes arguments into NodeSelectionParams
    let mut params = NodeSelectionParams {
        num_nodes: None,
        gpus_per_node: None,
        total_gpus: None,
        min_gpus_per_node: None,
        ib_only: false,
        prefer_same_block: true,
    };

    let mut topology_rule: Option<String> = None;
    let mut i = 0;
    while i < hermes_args.len() {
        match hermes_args[i].as_str() {
            "--num-nodes" => {
                params.num_nodes = Some(hermes_args[i + 1].parse()?);
                i += 2;
            }
            "--gpus-per-node" => {
                params.gpus_per_node = Some(hermes_args[i + 1].parse()?);
                i += 2;
            }
            "--total-gpus" => {
                params.total_gpus = Some(hermes_args[i + 1].parse()?);
                i += 2;
            }
            "--min-gpus-per-node" => {
                params.min_gpus_per_node = Some(hermes_args[i + 1].parse()?);
                i += 2;
            }
            "--topology-rule" => {
                topology_rule = Some(hermes_args[i + 1].clone());
                i += 2;
            }
            "--ib-only" => {
                params.ib_only = true;
                i += 1;
            }
            "--prefer-same-block" => {
                params.prefer_same_block = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    // run select-nodes to get topology values
    if let Ok(context) = std::env::var("HELM_KUBECONTEXT") {
        eprintln!("Analyzing cluster topology for context: {}", context);
    } else {
        eprintln!("Running hermes select-nodes...");
    }

    if is_debug {
        eprintln!(
            "DEBUG: Node selection params: num_nodes={:?}, gpus_per_node={:?}, total_gpus={:?}, ib_only={}, prefer_same_block={}",
            params.num_nodes,
            params.gpus_per_node,
            params.total_gpus,
            params.ib_only,
            params.prefer_same_block
        );
    }

    // scan cluster
    let scan_options = ScanOptions {
        format: "json".to_string(),
        node_filter: NodeFilter::All,
        detail_level: LabelDetailLevel::Basic,
        show_usage: false,
        save_to: None,
        cache_mode: CacheMode::UseCache,
        cache_ttl: None,
        topology_rule: topology_rule.clone(),
    };

    // initialize kubernetes client with helm-provided configuration
    let mut config = Config::infer().await?;

    // apply helm environment variable overrides
    // helm sets these when global flags are used (e.g., --kube-apiserver, --kube-insecure-skip-tls-verify, etc.)
    if let Ok(api_server) = std::env::var("HELM_KUBEAPISERVER") {
        if is_debug {
            eprintln!("DEBUG: Using HELM_KUBEAPISERVER: {}", api_server);
        }
        config.cluster_url = api_server.parse()?;
    }

    if let Ok(token) = std::env::var("HELM_KUBETOKEN") {
        if is_debug {
            eprintln!("DEBUG: Using HELM_KUBETOKEN for authentication");
        }
        config.auth_info.token = Some(token.into());
    }

    if let Ok(ca_file) = std::env::var("HELM_KUBECAFILE") {
        if is_debug {
            eprintln!("DEBUG: Using HELM_KUBECAFILE: {}", ca_file);
        }
        config.root_cert = Some(vec![std::fs::read(ca_file)?]);
    }

    // check TLS skip verify from multiple sources
    if std::env::var("HELM_KUBEINSECURE_SKIP_TLS_VERIFY").is_ok()
        || std::env::var("KUBE_INSECURE_TLS").is_ok()
        || std::env::var("KUBERNETES_INSECURE_TLS").is_ok()
    {
        if is_debug {
            eprintln!("DEBUG: TLS certificate verification disabled");
        }
        config.accept_invalid_certs = true;
    }

    let client = Client::try_from(config.clone())?;

    let cache_manager = CacheManager::new()?;
    let context_key = CacheManager::generate_context_key(&config);

    let cluster_report = if scan_options.cache_mode.should_use_cache()
        && let Some(cached_report) = cache_manager.load(&context_key, scan_options.cache_ttl)?
    {
        cached_report
    } else {
        let nodes: Api<Node> = Api::all(client.clone());
        let node_list = nodes.list(&ListParams::default()).await?;

        let platform_type = if let Some(first_node) = node_list.items.first() {
            let empty_labels = BTreeMap::new();
            let labels = first_node.metadata.labels.as_ref().unwrap_or(&empty_labels);
            detect_platform_from_labels(labels).get_platform_type()
        } else {
            PlatformType::GenericKubernetes
        };

        let cluster_topology_strategy =
            ClusterAnalyzer::determine_cluster_topology_strategy(&node_list.items, &platform_type);

        let mut cluster_report = ClusterReport {
            total_nodes: node_list.items.len(),
            rdma_nodes: 0,
            platform_type,
            api_server_url: config.cluster_url.to_string(),
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
                scan_options.detail_level,
                &cluster_topology_strategy,
                scan_options.topology_rule.as_deref(),
            )?;

            if cluster_report.topology_detection.is_none() {
                cluster_report.topology_detection = cluster_topology_strategy.clone();
            }

            if node_info.rdma_capability.is_capable() {
                cluster_report.rdma_nodes += 1;
            }

            cluster_report.nodes.push(node_info);
        }

        if scan_options.cache_mode.should_use_cache() {
            cache_manager.save(&context_key, &cluster_report)?;
        }

        cluster_report
    };

    // select nodes
    use hermes::node_selector::select_nodes_from_report;
    let selection = select_nodes_from_report(&cluster_report, &params)?;

    if is_debug {
        eprintln!("DEBUG: Selected {} nodes", selection.nodes.len());
    }

    let helm_values = selection.to_helm_values()?;

    // write to temp file in plugin dir (or system temp as fallback)
    let temp_dir = std::env::var("HELM_PLUGIN_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let temp_file = temp_dir.join(format!("hermes-topology-{}.yaml", uuid::Uuid::new_v4()));
    std::fs::write(&temp_file, &helm_values)?;

    if is_debug {
        eprintln!("DEBUG: Wrote topology values to: {}", temp_file.display());
    }

    eprintln!("Topology values generated:");
    eprintln!("{}", helm_values);
    eprintln!();

    // use HELM_BIN environment variable (set by helm when calling plugins)
    let helm_bin = std::env::var("HELM_BIN").unwrap_or_else(|_| "helm".to_string());

    // respect HELM_NAMESPACE if present and not already specified in helm_args
    if let Ok(namespace) = std::env::var("HELM_NAMESPACE")
        && !helm_args.iter().any(|a| a == "--namespace" || a == "-n")
    {
        helm_args.push("--namespace".to_string());
        helm_args.push(namespace.clone());
        if is_debug {
            eprintln!(
                "DEBUG: Auto-added namespace from HELM_NAMESPACE: {}",
                namespace
            );
        }
    }

    // build helm command
    eprintln!(
        "Running: {} {} {} {} -f {} {}",
        helm_bin,
        helm_cmd,
        release_name,
        chart,
        temp_file.display(),
        helm_args.join(" ")
    );

    // exec helm
    let mut cmd = Command::new(&helm_bin);
    cmd.arg(&helm_cmd)
        .arg(&release_name)
        .arg(&chart)
        .arg("-f")
        .arg(&temp_file)
        .args(&helm_args);

    if is_debug {
        eprintln!("DEBUG: Executing command: {:?}", cmd);
    }

    let status = cmd.status()?;

    // cleanup temp file
    let _ = std::fs::remove_file(&temp_file);

    if !status.success() {
        bail!("helm command failed with status: {}", status);
    }

    Ok(())
}
