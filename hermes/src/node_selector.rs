use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::{ClusterReport, NodeInfo};
use crate::topology_selector::get_topology_selector;

#[derive(Debug, Clone)]
pub struct NodeSelectionParams {
    pub num_nodes: Option<usize>,
    pub gpus_per_node: Option<u32>,
    pub total_gpus: Option<u32>,
    pub min_gpus_per_node: Option<u32>,
    pub ib_only: bool,
    pub prefer_same_block: bool,
}

impl NodeSelectionParams {
    /// resolve parameters into (num_nodes, gpus_per_node)
    /// returns special case (0, total_gpus) when only total_gpus is specified
    pub fn resolve(&self) -> Result<(usize, Option<u32>)> {
        match (self.num_nodes, self.total_gpus, self.gpus_per_node) {
            // explicit node count + GPUs per node
            (Some(n), None, Some(g)) => Ok((n, Some(g))),

            // total GPUs + GPUs per node = derive node count
            (None, Some(total), Some(g)) => {
                if total % g != 0 {
                    bail!(
                        "total-gpus ({}) not evenly divisible by gpus-per-node ({})",
                        total,
                        g
                    );
                }
                Ok((total as usize / g as usize, Some(g)))
            }

            // just node count, any GPU count
            (Some(n), None, None) => Ok((n, None)),

            // just total GPUs, let selector pick optimal split
            (None, Some(total), None) => {
                Ok((0, Some(total))) // special case: 0 means "optimize"
            }

            // default: 2 nodes for simple testing
            (None, None, None) => Ok((2, None)),

            // gpus_per_node alone doesn't make sense
            (None, None, Some(_)) => {
                bail!("--gpus-per-node requires either --num-nodes or --total-gpus")
            }

            // conflicting params
            (Some(_), Some(_), _) => {
                bail!("Cannot specify both --num-nodes and --total-gpus")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedNode {
    pub name: String,
    pub rdma_resource: String,
    pub gpus: u32,
    pub topology_block: Option<String>,
    pub rank: usize,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionSummary {
    pub total_nodes: usize,
    pub total_gpus: u32,
    pub gpus_per_node: u32,
    pub world_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyInfo {
    pub all_same_block: bool,
    pub blocks: HashMap<String, usize>,
    pub selection_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSelection {
    pub nodes: Vec<SelectedNode>,
    pub summary: SelectionSummary,
    pub topology: TopologyInfo,
    pub platform: String,
    pub rdma_type: String,
}

impl NodeSelection {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn to_shell(&self) -> String {
        let mut output = String::new();

        // comma-separated list of nodes
        let nodes_csv = self
            .nodes
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>()
            .join(",");

        output.push_str(&format!("export HERMES_NODES=\"{}\"\n", nodes_csv));
        output.push_str(&format!(
            "export HERMES_NUM_NODES={}\n",
            self.summary.total_nodes
        ));
        output.push_str(&format!(
            "export HERMES_TOTAL_GPUS={}\n",
            self.summary.total_gpus
        ));
        output.push_str(&format!(
            "export HERMES_GPUS_PER_NODE={}\n",
            self.summary.gpus_per_node
        ));
        output.push_str(&format!(
            "export HERMES_WORLD_SIZE={}\n",
            self.summary.world_size
        ));
        output.push_str(&format!("export HERMES_RDMA_TYPE=\"{}\"\n", self.rdma_type));
        output.push_str(&format!("export HERMES_PLATFORM=\"{}\"\n", self.platform));

        if let Some(first_block) = self.nodes.first().and_then(|n| n.topology_block.as_ref()) {
            output.push_str(&format!(
                "export HERMES_TOPOLOGY_BLOCK=\"{}\"\n",
                first_block
            ));
        }

        output.push_str(&format!(
            "export HERMES_ALL_SAME_BLOCK={}\n",
            self.topology.all_same_block
        ));

        // individual node variables for iteration
        for (i, node) in self.nodes.iter().enumerate() {
            output.push_str(&format!("export HERMES_NODE_{}=\"{}\"\n", i, node.name));
        }

        output
    }

    pub fn to_helm_values(&self) -> Result<String> {
        use serde_yaml::Value;

        let mut nodes_yaml = Vec::new();
        for node in &self.nodes {
            let mut node_map = serde_yaml::Mapping::new();
            node_map.insert(
                Value::String("name".to_string()),
                Value::String(node.name.clone()),
            );
            node_map.insert(
                Value::String("gpus".to_string()),
                Value::Number(node.gpus.into()),
            );
            node_map.insert(
                Value::String("rank".to_string()),
                Value::Number(node.rank.into()),
            );
            if let Some(block) = &node.topology_block {
                node_map.insert(
                    Value::String("topologyBlock".to_string()),
                    Value::String(block.clone()),
                );
            }
            nodes_yaml.push(Value::Mapping(node_map));
        }

        let mut summary_map = serde_yaml::Mapping::new();
        summary_map.insert(
            Value::String("totalNodes".to_string()),
            Value::Number(self.summary.total_nodes.into()),
        );
        summary_map.insert(
            Value::String("totalGpus".to_string()),
            Value::Number(self.summary.total_gpus.into()),
        );
        summary_map.insert(
            Value::String("gpusPerNode".to_string()),
            Value::Number(self.summary.gpus_per_node.into()),
        );
        summary_map.insert(
            Value::String("worldSize".to_string()),
            Value::Number(self.summary.world_size.into()),
        );

        let mut topology_map = serde_yaml::Mapping::new();
        topology_map.insert(
            Value::String("nodes".to_string()),
            Value::Sequence(nodes_yaml),
        );
        topology_map.insert(
            Value::String("summary".to_string()),
            Value::Mapping(summary_map),
        );
        topology_map.insert(
            Value::String("rdmaType".to_string()),
            Value::String(self.rdma_type.clone()),
        );
        topology_map.insert(
            Value::String("platform".to_string()),
            Value::String(self.platform.clone()),
        );
        topology_map.insert(
            Value::String("allSameBlock".to_string()),
            Value::Bool(self.topology.all_same_block),
        );
        if let Some(first_block) = self.nodes.first().and_then(|n| n.topology_block.as_ref()) {
            topology_map.insert(
                Value::String("topologyBlock".to_string()),
                Value::String(first_block.clone()),
            );
        }

        let mut root = serde_yaml::Mapping::new();
        root.insert(
            Value::String("topology".to_string()),
            Value::Mapping(topology_map),
        );

        Ok(serde_yaml::to_string(&Value::Mapping(root))?)
    }
}

/// select nodes from cluster report using platform-specific topology logic
pub fn select_nodes_from_report(
    report: &ClusterReport,
    params: &NodeSelectionParams,
) -> Result<NodeSelection> {
    let (mut num_nodes, gpus_per_node) = params.resolve()?;

    // filter RDMA-capable nodes
    let mut candidates: Vec<NodeInfo> = report
        .nodes
        .iter()
        .filter(|n| n.rdma_capability.is_capable())
        .cloned()
        .collect();

    if candidates.is_empty() {
        bail!("No RDMA-capable nodes found in cluster");
    }

    // apply filters
    if params.ib_only {
        candidates.retain(|node| {
            node.rdma_resource
                .as_ref()
                .map(|r| r.contains("ib") || r.contains("IB"))
                .unwrap_or(false)
        });
    }

    if let Some(min_gpus) = params.min_gpus_per_node {
        candidates.retain(|node| node.gpu_count.unwrap_or(0) >= min_gpus);
    }

    // handle GPU-based selection
    if let Some(gpus) = gpus_per_node {
        if num_nodes == 0 {
            // optimize mode: find best node/GPU split
            let mut by_gpu_count: HashMap<u32, Vec<&NodeInfo>> = HashMap::new();
            for node in &candidates {
                if let Some(gpu_count) = node.gpu_count {
                    by_gpu_count.entry(gpu_count).or_default().push(node);
                }
            }

            // prefer fewer nodes with more GPUs
            let mut gpu_counts: Vec<u32> = by_gpu_count.keys().copied().collect();
            gpu_counts.sort_by(|a, b| b.cmp(a));

            let mut best_split: Option<(usize, u32)> = None;
            for gpu_count in gpu_counts {
                if gpus % gpu_count == 0 {
                    let needed_nodes = (gpus / gpu_count) as usize;
                    let available = by_gpu_count.get(&gpu_count).unwrap().len();

                    if available >= needed_nodes {
                        best_split = Some((needed_nodes, gpu_count));
                        break;
                    }
                }
            }

            if let Some((n, g)) = best_split {
                num_nodes = n;
                candidates.retain(|node| node.gpu_count.unwrap_or(0) == g);
            } else {
                bail!("Cannot satisfy {} total GPUs with available nodes", gpus);
            }
        } else {
            candidates.retain(|node| node.gpu_count.unwrap_or(0) == gpus);
        }
    }

    if candidates.is_empty() {
        bail!("No nodes match the specified criteria");
    }

    // use platform-specific topology selector
    let selector = get_topology_selector(&report.platform_type);

    // group by topology using platform selector
    let mut topology_groups: HashMap<String, Vec<&NodeInfo>> = HashMap::new();
    for node in &candidates {
        if let Some(topology_key) = selector.get_topology_key(node) {
            topology_groups.entry(topology_key).or_default().push(node);
        } else {
            topology_groups
                .entry("unknown".to_string())
                .or_default()
                .push(node);
        }
    }

    // select nodes using topology-aware logic
    let selected_refs: Vec<&NodeInfo> = if params.prefer_same_block {
        // find largest topology group that can satisfy our needs
        let best_group = topology_groups
            .iter()
            .filter(|(_, nodes)| nodes.len() >= num_nodes)
            .max_by_key(|(_, nodes)| nodes.len())
            .map(|(_, nodes)| nodes);

        if let Some(group_nodes) = best_group {
            group_nodes.iter().copied().take(num_nodes).collect()
        } else {
            // fallback: take from largest groups until we have enough
            let mut selected = Vec::new();
            let mut sorted_groups: Vec<_> = topology_groups.values().collect();
            sorted_groups.sort_by_key(|nodes| std::cmp::Reverse(nodes.len()));

            for group in sorted_groups {
                for node in group.iter() {
                    if selected.len() < num_nodes {
                        selected.push(*node);
                    }
                }
                if selected.len() >= num_nodes {
                    break;
                }
            }
            selected
        }
    } else {
        // just take first N nodes
        candidates.iter().take(num_nodes).collect()
    };

    if selected_refs.len() < num_nodes {
        bail!(
            "Only found {} nodes matching criteria, need {}",
            selected_refs.len(),
            num_nodes
        );
    }

    // convert to SelectedNode
    let selected: Vec<SelectedNode> = selected_refs
        .iter()
        .enumerate()
        .map(|(rank, node)| SelectedNode {
            name: node.name.clone(),
            rdma_resource: node.rdma_resource.clone().unwrap_or_default(),
            gpus: node.gpu_count.unwrap_or(0),
            topology_block: selector.get_topology_key(node),
            rank,
            labels: node.node_labels.clone(),
        })
        .collect();

    // build summary
    let total_nodes = selected.len();
    let total_gpus: u32 = selected.iter().map(|n| n.gpus).sum();
    let gpus_per_node_actual = if selected.iter().all(|n| n.gpus == selected[0].gpus) {
        selected[0].gpus
    } else {
        total_gpus / total_nodes as u32
    };
    let world_size = total_gpus;

    let summary = SelectionSummary {
        total_nodes,
        total_gpus,
        gpus_per_node: gpus_per_node_actual,
        world_size,
    };

    // analyze topology
    let mut blocks: HashMap<String, usize> = HashMap::new();
    for node in &selected {
        let block = node
            .topology_block
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        *blocks.entry(block).or_default() += 1;
    }

    let all_same_block = blocks.len() == 1 && !blocks.contains_key("unknown");

    let selection_reason = if all_same_block {
        let block_name = blocks.keys().next().unwrap();
        format!(
            "All {} nodes in same topology block '{}' for optimal locality",
            total_nodes, block_name
        )
    } else if blocks.len() == 1 {
        format!("All {} nodes selected (topology unknown)", total_nodes)
    } else {
        format!(
            "{} nodes across {} topology blocks",
            total_nodes,
            blocks.len()
        )
    };

    let topology = TopologyInfo {
        all_same_block,
        blocks,
        selection_reason,
    };

    let rdma_type = selected[0].rdma_resource.clone();
    let platform = format!("{:?}", report.platform_type);

    Ok(NodeSelection {
        nodes: selected,
        summary,
        topology,
        platform,
        rdma_type,
    })
}
