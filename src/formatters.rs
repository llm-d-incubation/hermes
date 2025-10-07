use crate::models::*;
use anyhow::Result;
use prettytable::{Cell, Row, Table, format};

pub trait ReportFormatter {
    fn format_report(&self, report: &ClusterReport) -> Result<String>;
}

pub struct JsonFormatter;
pub struct YamlFormatter;
pub struct TableFormatter;

impl ReportFormatter for JsonFormatter {
    fn format_report(&self, report: &ClusterReport) -> Result<String> {
        Ok(serde_json::to_string_pretty(report)?)
    }
}

impl ReportFormatter for YamlFormatter {
    fn format_report(&self, report: &ClusterReport) -> Result<String> {
        Ok(serde_yaml::to_string(report)?)
    }
}

impl ReportFormatter for TableFormatter {
    fn format_report(&self, report: &ClusterReport) -> Result<String> {
        let mut output = String::new();

        // summary table with nice formatting
        let summary_table = self.create_summary_table(report);
        output.push_str(&format!("{}\n", summary_table));

        // topology blocks table
        if !report.topology_blocks.is_empty() {
            output.push_str("\nTopology Distribution:\n");
            let topo_table = self.create_topology_table(report);
            output.push_str(&format!("{}\n", topo_table));
        }

        // GPU distribution table
        if !report.topology_gpu_counts.is_empty() {
            let topology_type_name = self.get_topology_type_name(report);
            output.push_str(&format!("\nGPU Distribution by {}:\n", topology_type_name));
            let gpu_table = self.create_gpu_distribution_table(report, &topology_type_name);
            output.push_str(&format!("{}\n", gpu_table));
        }

        // sr-iov networks table
        if !report.sriov_networks.is_empty() {
            output.push_str("\nSR-IOV Networks:\n");
            let sriov_table = self.create_sriov_networks_table(report);
            output.push_str(&format!("{}\n", sriov_table));
        }

        // node details table
        if !report.nodes.is_empty() {
            output.push_str("\nNode Details:\n");
            let node_table = self.create_node_details_table(report);
            output.push_str(&format!("{}\n", node_table));
        }

        Ok(output)
    }
}

impl TableFormatter {
    fn create_summary_table(&self, report: &ClusterReport) -> Table {
        let mut summary_table = Table::new();
        summary_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        let titles_row = Row::new(vec![
            Cell::new("Metric").style_spec("Fb"),
            Cell::new("Value").style_spec("Fb"),
        ]);
        summary_table.set_titles(titles_row);

        summary_table.add_row(Row::new(vec![
            Cell::new("Platform Type").style_spec("c"),
            Cell::new(&report.platform_type.to_string()).style_spec("Fc"),
        ]));

        if let Some(ref detection) = report.topology_detection {
            summary_table.add_row(Row::new(vec![
                Cell::new("Topology Type"),
                Cell::new(&detection.topology_type.to_string()).style_spec("Fc"),
            ]));
            summary_table.add_row(Row::new(vec![
                Cell::new("Detection Method"),
                Cell::new(&detection.detection_method),
            ]));
            summary_table.add_row(Row::new(vec![
                Cell::new("Detection Confidence"),
                Cell::new(&detection.confidence).style_spec("Fy"),
            ]));
        }

        summary_table.add_row(Row::new(vec![
            Cell::new("Total Nodes"),
            Cell::new(&report.total_nodes.to_string()).style_spec("Fr"),
        ]));
        summary_table.add_row(Row::new(vec![
            Cell::new("RDMA-Capable Nodes"),
            Cell::new(&format!(
                "{} ({:.1}%)",
                report.rdma_nodes,
                (report.rdma_nodes as f32 / report.total_nodes as f32) * 100.0
            ))
            .style_spec("Fg"),
        ]));

        if !report.rdma_types.is_empty() {
            let rdma_types_clean: Vec<&String> =
                report.rdma_types.iter().filter(|s| !s.is_empty()).collect();
            if !rdma_types_clean.is_empty() {
                summary_table.add_row(Row::new(vec![
                    Cell::new("RDMA Types"),
                    Cell::new(
                        &rdma_types_clean
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
        }

        summary_table.add_row(Row::new(vec![
            Cell::new("GPU Nodes"),
            Cell::new(&format!(
                "{} ({:.1}%)",
                report.gpu_nodes,
                (report.gpu_nodes as f32 / report.total_nodes as f32) * 100.0
            ))
            .style_spec("Fy"),
        ]));
        summary_table.add_row(Row::new(vec![
            Cell::new("Total GPUs"),
            Cell::new(&report.total_gpus.to_string()).style_spec("Fr"),
        ]));

        self.add_optional_summary_rows(&mut summary_table, report);

        summary_table
    }

    fn add_optional_summary_rows(&self, table: &mut Table, report: &ClusterReport) {
        if !report.gpu_types.is_empty() {
            let gpu_types_clean: Vec<&String> =
                report.gpu_types.iter().filter(|s| !s.is_empty()).collect();
            if !gpu_types_clean.is_empty() {
                table.add_row(Row::new(vec![
                    Cell::new("GPU Types"),
                    Cell::new(
                        &gpu_types_clean
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
        }

        if !report.ib_fabrics.is_empty() {
            let ib_fabrics_clean: Vec<&String> =
                report.ib_fabrics.iter().filter(|s| !s.is_empty()).collect();
            if !ib_fabrics_clean.is_empty() {
                table.add_row(Row::new(vec![
                    Cell::new("InfiniBand Fabrics"),
                    Cell::new(
                        &ib_fabrics_clean
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
        }

        if !report.superpods.is_empty() {
            let superpods_clean: Vec<&String> =
                report.superpods.iter().filter(|s| !s.is_empty()).collect();
            if !superpods_clean.is_empty() {
                table.add_row(Row::new(vec![
                    Cell::new("Superpods"),
                    Cell::new(
                        &superpods_clean
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
        }

        if !report.leafgroups.is_empty() {
            let leafgroups_clean: Vec<&String> =
                report.leafgroups.iter().filter(|s| !s.is_empty()).collect();
            if !leafgroups_clean.is_empty() {
                table.add_row(Row::new(vec![
                    Cell::new("Leaf Groups"),
                    Cell::new(
                        &leafgroups_clean
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]));
            }
        }
    }

    fn create_topology_table(&self, report: &ClusterReport) -> Table {
        let mut topo_table = Table::new();
        topo_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        let titles_row = Row::new(vec![
            Cell::new("Topology Block").style_spec("Fb"),
            Cell::new("Node Count").style_spec("Fb"),
            Cell::new("Percentage").style_spec("Fb"),
        ]);
        topo_table.set_titles(titles_row);

        for (block, count) in &report.topology_blocks {
            let percentage = (*count as f32 / report.total_nodes as f32) * 100.0;
            topo_table.add_row(Row::new(vec![
                Cell::new(block),
                Cell::new(&count.to_string()).style_spec("Fr"),
                Cell::new(&format!("{:.1}%", percentage)).style_spec("Fc"),
            ]));
        }

        topo_table
    }

    fn get_topology_type_name(&self, report: &ClusterReport) -> String {
        if let Some(ref detection) = report.topology_detection {
            match detection.topology_type {
                TopologyType::LeafGroup => "Fabric".to_string(), // CoreWeave leafgroups aggregate to fabric
                _ => detection.topology_type.to_string(),
            }
        } else {
            "Topology Group".to_string()
        }
    }

    fn create_gpu_distribution_table(
        &self,
        report: &ClusterReport,
        topology_type_name: &str,
    ) -> Table {
        let mut gpu_table = Table::new();
        gpu_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        let titles_row = Row::new(vec![
            Cell::new(topology_type_name).style_spec("Fb"),
            Cell::new("GPU Count").style_spec("Fb"),
            Cell::new("Percentage").style_spec("Fb"),
        ]);
        gpu_table.set_titles(titles_row);

        let all_gpu_entries = self.collect_gpu_entries(report);

        for (block, gpu_count) in all_gpu_entries {
            let percentage = if report.total_gpus > 0 {
                (gpu_count as f32 / report.total_gpus as f32) * 100.0
            } else {
                0.0
            };
            let gpu_cell = if gpu_count > 0 {
                Cell::new(&gpu_count.to_string()).style_spec("Fy")
            } else {
                Cell::new("0").style_spec("Fd")
            };
            gpu_table.add_row(Row::new(vec![
                Cell::new(&block),
                gpu_cell,
                Cell::new(&format!("{:.1}%", percentage)).style_spec("Fc"),
            ]));
        }

        // add total row
        gpu_table.add_row(Row::new(vec![
            Cell::new("TOTAL").style_spec("Fb"),
            Cell::new(&report.total_gpus.to_string()).style_spec("FbY"),
            Cell::new("100.0%").style_spec("FbG"),
        ]));

        gpu_table
    }

    fn collect_gpu_entries(&self, report: &ClusterReport) -> Vec<(String, u32)> {
        let mut all_gpu_entries: Vec<(String, u32)> = Vec::new();

        // for CoreWeave, we need to list all fabric entries, not topology blocks
        if report.platform_type == PlatformType::CoreWeave {
            // collect all unique fabric names from GPU counts and ib_fabrics
            let mut all_fabrics = std::collections::HashSet::new();
            for fabric in report.topology_gpu_counts.keys() {
                all_fabrics.insert(fabric.clone());
            }
            for fabric in &report.ib_fabrics {
                all_fabrics.insert(fabric.clone());
            }

            for fabric in all_fabrics {
                let gpu_count = report
                    .topology_gpu_counts
                    .get(&fabric)
                    .copied()
                    .unwrap_or(0);
                all_gpu_entries.push((fabric, gpu_count));
            }
        } else {
            // for other platforms, use topology blocks
            for block in report.topology_blocks.keys() {
                let gpu_count = report.topology_gpu_counts.get(block).copied().unwrap_or(0);
                all_gpu_entries.push((block.clone(), gpu_count));
            }
        }

        // sort by GPU count (descending) for better readability
        all_gpu_entries.sort_by(|a, b| b.1.cmp(&a.1));
        all_gpu_entries
    }

    fn create_sriov_networks_table(&self, report: &ClusterReport) -> Table {
        let mut sriov_table = Table::new();
        sriov_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        let titles_row = Row::new(vec![
            Cell::new("Name").style_spec("Fb"),
            Cell::new("Target Namespace").style_spec("Fb"),
            Cell::new("Resource Name").style_spec("Fb"),
            Cell::new("VLAN").style_spec("Fb"),
        ]);
        sriov_table.set_titles(titles_row);

        for network in &report.sriov_networks {
            sriov_table.add_row(Row::new(vec![
                Cell::new(&network.name).style_spec("Fg"),
                Cell::new(&network.namespace).style_spec("Fc"),
                Cell::new(&network.resource_name).style_spec("Fy"),
                Cell::new(
                    &network
                        .vlan
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ]));
        }

        sriov_table
    }

    fn create_node_details_table(&self, report: &ClusterReport) -> Table {
        let mut node_table = Table::new();
        node_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        let topology_header = if let Some(ref detection) = report.topology_detection {
            format!("Topology ({})", detection.topology_type)
        } else {
            "Topology".to_string()
        };

        let title_cells = self.create_node_table_headers(&topology_header, report);
        let titles_row = Row::new(title_cells);
        node_table.set_titles(titles_row);

        for node in &report.nodes {
            let row_cells = self.create_node_table_row(node, report);
            node_table.add_row(Row::new(row_cells));
        }

        node_table
    }

    fn create_node_table_headers(
        &self,
        topology_header: &str,
        report: &ClusterReport,
    ) -> Vec<Cell> {
        let mut title_cells = vec![
            Cell::new("Node Name").style_spec("Fb"),
            Cell::new("RDMA").style_spec("Fb"),
            Cell::new("RDMA Type").style_spec("Fb"),
            Cell::new("Platform").style_spec("Fb"),
            Cell::new(topology_header).style_spec("Fb"),
        ];

        // add platform-specific columns
        if report.platform_type == PlatformType::CoreWeave {
            title_cells.extend(vec![
                Cell::new("IB Speed").style_spec("Fb"),
                Cell::new("Fabric").style_spec("Fb"),
            ]);
        } else if report.platform_type == PlatformType::GKE {
            title_cells.push(Cell::new("Node Pool").style_spec("Fb"));
        }

        title_cells.extend(vec![
            Cell::new("GPU Count").style_spec("Fb"),
            Cell::new("GPU Type").style_spec("Fb"),
        ]);

        title_cells
    }

    fn create_node_table_row(&self, node: &NodeInfo, report: &ClusterReport) -> Vec<Cell> {
        let rdma_cell = if node.rdma_capable {
            Cell::new("Yes").style_spec("Fg")
        } else {
            Cell::new("No").style_spec("Fr")
        };

        let mut row_cells = vec![
            Cell::new(&node.name),
            rdma_cell,
            Cell::new(node.rdma_type.as_deref().unwrap_or("-")),
            Cell::new(&node.platform_type.to_string()),
            Cell::new(node.topology_block.as_deref().unwrap_or("-")),
        ];

        // add platform-specific columns
        if report.platform_type == PlatformType::CoreWeave {
            row_cells.extend(vec![
                Cell::new(node.ib_speed.as_deref().unwrap_or("-")),
                Cell::new(node.ib_fabric.as_deref().unwrap_or("-")),
            ]);
        } else if report.platform_type == PlatformType::GKE {
            row_cells.push(Cell::new(node.gke_nodepool.as_deref().unwrap_or("-")));
        }

        row_cells.extend(vec![
            Cell::new(
                &node
                    .gpu_count
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::new(node.gpu_type.as_deref().unwrap_or("-")),
        ]);

        row_cells
    }
}

pub fn get_formatter(format: &str) -> Box<dyn ReportFormatter> {
    match format {
        "json" => Box::new(JsonFormatter),
        "yaml" => Box::new(YamlFormatter),
        _ => Box::new(TableFormatter),
    }
}
