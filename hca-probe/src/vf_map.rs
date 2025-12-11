//! Map SR-IOV Virtual Functions to their Physical Functions

use crate::OutputFormat;
use crate::sysfs::{self, NetInterface};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct PfMapping {
    pf: String,
    vfs: Vec<VfEntry>,
}

#[derive(Debug, Serialize)]
struct VfEntry {
    name: Option<String>,
    virtfn_index: String,
}

#[derive(Debug, Serialize)]
struct VfMapOutput {
    mappings: Vec<PfMapping>,
    total_pfs: usize,
    total_vfs: usize,
}

pub fn run(format: OutputFormat) -> Result<()> {
    let output = collect_vf_mappings()?;

    match format {
        OutputFormat::Table => print_table(&output),
        OutputFormat::Json => print_json(&output)?,
    }

    Ok(())
}

fn collect_vf_mappings() -> Result<VfMapOutput> {
    let interfaces = sysfs::list_net_interfaces()?;

    // find all PFs (has device, not a VF)
    let pfs: Vec<&NetInterface> = interfaces
        .iter()
        .filter(|i| i.has_device && !i.is_vf)
        .collect();

    let mut mappings = Vec::new();
    let mut total_vfs = 0;

    for pf in &pfs {
        let pf_path = Path::new(sysfs::NET_CLASS).join(&pf.name);
        let vf_infos = sysfs::get_vfs_for_pf(&pf_path);

        if vf_infos.is_empty() {
            continue;
        }

        let vfs: Vec<VfEntry> = vf_infos
            .into_iter()
            .map(|v| VfEntry {
                name: v.iface_name,
                virtfn_index: v.virtfn_index,
            })
            .collect();

        total_vfs += vfs.len();

        mappings.push(PfMapping {
            pf: pf.name.clone(),
            vfs,
        });
    }

    Ok(VfMapOutput {
        total_pfs: mappings.len(),
        total_vfs,
        mappings,
    })
}

fn print_table(output: &VfMapOutput) {
    println!("{:<12} | {}", "PF", "Mapped SR-IOV VFs");
    println!("{}", "=".repeat(60));

    if output.mappings.is_empty() {
        println!("(No SR-IOV PFs with VFs found)");
        return;
    }

    for mapping in &output.mappings {
        let vf_names: Vec<String> = mapping
            .vfs
            .iter()
            .map(|v| v.name.clone().unwrap_or_else(|| "<orphan>".to_string()))
            .collect();

        println!("{:<12} | {}", mapping.pf, vf_names.join(" "));
    }

    println!();
    println!(
        "Total: {} PF(s) with {} VF(s)",
        output.total_pfs, output.total_vfs
    );
}

fn print_json(output: &VfMapOutput) -> Result<()> {
    let json = serde_json::to_string_pretty(output)?;
    println!("{}", json);
    Ok(())
}
