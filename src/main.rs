use anyhow::Result;
use clap::Parser;
use k8s_openapi::api::core::v1::Node;
use kube::{Api, Client, Config, api::ListParams};
use std::collections::{BTreeMap, HashMap};
use tracing::info;

use hermes::analyzer::ClusterAnalyzer;
use hermes::cache::CacheManager;
use hermes::crds::sriovnetworks::SriovNetwork;
use hermes::formatters::*;
use hermes::models::*;
use hermes::platforms::*;

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

        /// Save scan results to file for use by self-test
        #[arg(long)]
        save_to: Option<String>,

        /// Skip cache and force fresh scan
        #[arg(long)]
        no_cache: bool,

        /// Cache TTL in hours (default: 24)
        #[arg(long)]
        cache_ttl: Option<i64>,
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
    },
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
            save_to,
            no_cache,
            cache_ttl,
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

            run_scan(
                format,
                node_filter,
                detail_level,
                save_to,
                cache_mode,
                cache_ttl,
            )
            .await
        }
        Commands::SelfTest {
            namespace,
            from_stdin,
            no_cleanup,
            load_from,
            dry_run,
            sriov_network,
            request_gpu,
            no_cleanup_on_signal,
            workload,
            image,
            gpus_per_node,
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
            };

            run_self_test(config).await
        }
    }
}

async fn run_scan(
    format: String,
    node_filter: NodeFilter,
    detail_level: LabelDetailLevel,
    save_to: Option<String>,
    cache_mode: CacheMode,
    cache_ttl: Option<i64>,
) -> Result<()> {
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

    if cache_mode.should_use_cache()
        && let Some(cached_report) = cache_manager.load(&context_key, cache_ttl)?
    {
        // use cached report
        let mut cluster_report = cached_report;

        // apply RDMA filter if requested
        if matches!(node_filter, NodeFilter::RdmaOnly) {
            cluster_report
                .nodes
                .retain(|node| node.rdma_capability.is_capable());
        }

        // save to file if requested
        if let Some(save_path) = &save_to {
            info!("Saving scan results to: {}", save_path);
            let json_data = serde_json::to_string_pretty(&cluster_report)?;
            std::fs::write(save_path, json_data)?;
            println!("Scan results saved to: {}", save_path);
        }

        let formatter = get_formatter(&format);
        let output = formatter.format_report(&cluster_report)?;

        if format == "table" {
            print!("{}", output);
        } else {
            println!("{}", output);
        }

        return Ok(());
    }

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
        nodes: Vec::new(),
        gpu_nodes: 0,
        gpu_types: Vec::new(),
        total_gpus: 0,
        image_checked: None,
        cache_check_timestamp: None,
    };

    for node in node_list.items {
        let node_info =
            ClusterAnalyzer::analyze_node(&node, detail_level, &cluster_topology_strategy)?;

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

        match node_filter {
            NodeFilter::RdmaOnly if node_info.rdma_capability.is_capable() => {
                cluster_report.nodes.push(node_info);
            }
            NodeFilter::All => {
                cluster_report.nodes.push(node_info);
            }
            _ => {}
        }
    }

    // detect SR-IOV networks for OpenShift (not applicable to CoreWeave or GKE)
    if matches!(
        platform_type,
        PlatformType::OpenShift | PlatformType::GenericKubernetes
    ) {
        info!("Detecting SR-IOV networks across all namespaces...");
        cluster_report.sriov_networks = detect_sriov_networks(&client).await;
    }

    // save scan results to cache (unless cache mode is SkipCache)
    if cache_mode.should_use_cache() {
        cache_manager.save(&context_key, &cluster_report)?;
    }

    // save scan results to file if requested
    if let Some(save_path) = &save_to {
        info!("Saving scan results to: {}", save_path);
        let json_data = serde_json::to_string_pretty(&cluster_report)?;
        std::fs::write(save_path, json_data)?;
        println!("Scan results saved to: {}", save_path);
    }

    let formatter = get_formatter(&format);
    let output = formatter.format_report(&cluster_report)?;

    // only use println! for table format since it's already formatted
    // for json/yaml, the output already includes formatting
    if format == "table" {
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
        println!("Loaded scan data from: {}", scan_file);
        Some(report)
    } else {
        // try to load from cache
        let cache_manager = CacheManager::new()?;
        let config = Config::infer().await?;
        let context_key = CacheManager::generate_context_key(&config);

        if let Some(report) = cache_manager.load(&context_key, None)? {
            println!("Using cached cluster scan data");
            Some(report)
        } else {
            println!("Will perform fresh cluster scan for node selection");
            None
        }
    };

    // note: cached_scan can be used in future for intelligent node selection optimization

    if config.execution_mode.is_dry_run() {
        println!("Dry run mode: will render manifests to stdout without deploying");
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
