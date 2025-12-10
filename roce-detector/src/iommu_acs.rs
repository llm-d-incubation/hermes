//! Check IOMMU and PCI ACS configuration

use crate::OutputFormat;
use crate::sysfs;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

// PCI Extended Capability ID for ACS
const PCI_EXT_CAP_ID_ACS: u16 = 0x000D;
// offset within ACS capability to control register
const ACS_CTRL_OFFSET: usize = 6;

#[derive(Debug, Serialize)]
struct IommuStatus {
    cpu_vendor: String,
    iommu_enabled: bool,
    iommu_param: Option<String>,
    iommu_groups_present: bool,
    iommu_groups_count: usize,
}

#[derive(Debug, Serialize)]
struct AcsDevice {
    slot: String,
    has_acs_cap: bool,
    acs_enabled: bool,
    acs_ctrl: Option<u16>,
    device_class: Option<String>,
    iommu_group: Option<String>,
}

#[derive(Debug, Serialize)]
struct IommuAcsOutput {
    iommu: IommuStatus,
    acs_devices: Vec<AcsDevice>,
    summary: AcsSummary,
}

#[derive(Debug, Serialize)]
struct AcsSummary {
    total_pci_devices: usize,
    devices_with_acs: usize,
    acs_enabled_count: usize,
    acs_disabled_count: usize,
}

pub fn run(format: OutputFormat) -> Result<()> {
    let output = collect_iommu_acs()?;

    match format {
        OutputFormat::Table => print_table(&output),
        OutputFormat::Json => print_json(&output)?,
    }

    Ok(())
}

fn collect_iommu_acs() -> Result<IommuAcsOutput> {
    let iommu = detect_iommu_status()?;
    let (acs_devices, summary) = detect_acs_status()?;

    Ok(IommuAcsOutput {
        iommu,
        acs_devices,
        summary,
    })
}

fn detect_iommu_status() -> Result<IommuStatus> {
    // detect CPU vendor from /proc/cpuinfo
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu_vendor = cpuinfo
        .lines()
        .find(|l| l.starts_with("vendor_id"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // check kernel command line for IOMMU params
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();

    let (iommu_enabled, iommu_param) = if cpu_vendor.contains("Intel") {
        let param = find_param(&cmdline, "intel_iommu");
        let enabled = param.as_ref().map(|p| p.contains("on")).unwrap_or(false);
        (enabled, param)
    } else if cpu_vendor.contains("AMD") {
        let param = find_param(&cmdline, "amd_iommu");
        // AMD IOMMU is on by default, check if explicitly disabled
        let enabled = !param.as_ref().map(|p| p.contains("off")).unwrap_or(false);
        (enabled, param)
    } else {
        // generic check
        let intel = find_param(&cmdline, "intel_iommu");
        let amd = find_param(&cmdline, "amd_iommu");
        let param = intel.or(amd);
        let enabled = param.as_ref().map(|p| p.contains("on")).unwrap_or(false);
        (enabled, param)
    };

    // check if IOMMU groups exist (runtime indicator)
    let iommu_groups_path = Path::new("/sys/kernel/iommu_groups");
    let iommu_groups_present = iommu_groups_path.exists();
    let iommu_groups_count = if iommu_groups_present {
        fs::read_dir(iommu_groups_path)
            .map(|d| d.count())
            .unwrap_or(0)
    } else {
        0
    };

    Ok(IommuStatus {
        cpu_vendor,
        iommu_enabled: iommu_enabled || iommu_groups_count > 0,
        iommu_param,
        iommu_groups_present,
        iommu_groups_count,
    })
}

fn find_param(cmdline: &str, name: &str) -> Option<String> {
    for part in cmdline.split_whitespace() {
        if part.starts_with(name) {
            return Some(part.to_string());
        }
    }
    None
}

fn detect_acs_status() -> Result<(Vec<AcsDevice>, AcsSummary)> {
    let pci_devices = sysfs::list_pci_devices()?;
    let mut acs_devices = Vec::new();
    let mut devices_with_acs = 0;
    let mut acs_enabled_count = 0;
    let mut acs_disabled_count = 0;

    for device in &pci_devices {
        let config_path = device.path.join("config");
        if !config_path.exists() {
            continue;
        }

        // read PCI config space (need at least 256 bytes for standard, 4096 for extended)
        let config = match fs::read(&config_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (has_acs_cap, acs_enabled, acs_ctrl) = check_acs_capability(&config);

        if has_acs_cap {
            devices_with_acs += 1;
            if acs_enabled {
                acs_enabled_count += 1;
            } else {
                acs_disabled_count += 1;
            }
        }

        // only report devices that have ACS capability
        if has_acs_cap {
            let device_class = device.class.as_ref().map(|c| decode_device_class(c));

            acs_devices.push(AcsDevice {
                slot: device.slot.clone(),
                has_acs_cap,
                acs_enabled,
                acs_ctrl,
                device_class,
                iommu_group: device.iommu_group.clone(),
            });
        }
    }

    let summary = AcsSummary {
        total_pci_devices: pci_devices.len(),
        devices_with_acs,
        acs_enabled_count,
        acs_disabled_count,
    };

    Ok((acs_devices, summary))
}

/// Check for ACS capability in PCI config space
fn check_acs_capability(config: &[u8]) -> (bool, bool, Option<u16>) {
    // need at least standard 256 byte config
    if config.len() < 256 {
        return (false, false, None);
    }

    // extended capabilities start at offset 0x100 (256)
    // need extended config space for ACS
    if config.len() <= 0x100 {
        return (false, false, None);
    }

    // walk extended capability list
    let mut offset = 0x100;

    while offset > 0 && offset < config.len() - 4 {
        // extended cap header is 4 bytes: [cap_id:16, version:4, next:12]
        let header = u32::from_le_bytes([
            config[offset],
            config.get(offset + 1).copied().unwrap_or(0),
            config.get(offset + 2).copied().unwrap_or(0),
            config.get(offset + 3).copied().unwrap_or(0),
        ]);

        let cap_id = (header & 0xFFFF) as u16;
        let next_offset = ((header >> 20) & 0xFFF) as usize;

        if cap_id == PCI_EXT_CAP_ID_ACS {
            // found ACS capability
            // ACS control register is at cap + 6
            let ctrl_offset = offset + ACS_CTRL_OFFSET;
            if ctrl_offset + 1 < config.len() {
                let acs_ctrl = u16::from_le_bytes([
                    config[ctrl_offset],
                    config.get(ctrl_offset + 1).copied().unwrap_or(0),
                ]);

                // ACS is "enabled" if any of the control bits are set
                // bits 0-5 are the enable bits: SV, TB, RR, CR, UF, EC
                let acs_enabled = (acs_ctrl & 0x3F) != 0;

                return (true, acs_enabled, Some(acs_ctrl));
            }
            return (true, false, None);
        }

        if next_offset == 0 || next_offset <= offset {
            break;
        }
        offset = next_offset;
    }

    (false, false, None)
}

fn decode_device_class(class_code: &str) -> String {
    // class code is 0xXXYYZZ where XX=class, YY=subclass, ZZ=prog-if
    let code = class_code
        .trim_start_matches("0x")
        .parse::<u32>()
        .unwrap_or(0);

    let class = (code >> 16) & 0xFF;

    match class {
        0x01 => "Storage".to_string(),
        0x02 => "Network".to_string(),
        0x03 => "Display".to_string(),
        0x04 => "Multimedia".to_string(),
        0x05 => "Memory".to_string(),
        0x06 => "Bridge".to_string(),
        0x07 => "Communication".to_string(),
        0x08 => "System".to_string(),
        0x09 => "Input".to_string(),
        0x0C => "Serial".to_string(),
        0x0D => "Wireless".to_string(),
        0x12 => "Processing".to_string(),
        _ => format!("Class 0x{:02X}", class),
    }
}

fn print_table(output: &IommuAcsOutput) {
    println!("IOMMU Status");
    println!("{}", "=".repeat(50));
    println!("CPU Vendor:      {}", output.iommu.cpu_vendor);
    println!(
        "IOMMU Enabled:   {}",
        if output.iommu.iommu_enabled {
            "✓ Yes"
        } else {
            "✗ No"
        }
    );
    if let Some(ref param) = output.iommu.iommu_param {
        println!("Kernel Param:    {}", param);
    }
    println!(
        "IOMMU Groups:    {} groups present",
        output.iommu.iommu_groups_count
    );

    println!();
    println!("PCI ACS Status");
    println!("{}", "=".repeat(50));
    println!("Total PCI Devices:    {}", output.summary.total_pci_devices);
    println!("Devices with ACS:     {}", output.summary.devices_with_acs);
    println!("ACS Enabled:          {}", output.summary.acs_enabled_count);
    println!(
        "ACS Disabled:         {}",
        output.summary.acs_disabled_count
    );

    if !output.acs_devices.is_empty() {
        println!();
        println!(
            "{:<14} | {:<8} | {:<10} | {}",
            "PCI Slot", "ACS", "IOMMU Grp", "Class"
        );
        println!("{}", "-".repeat(50));

        for dev in &output.acs_devices {
            let acs_status = if dev.acs_enabled { "✓ On" } else { "✗ Off" };
            let iommu_grp = dev.iommu_group.as_deref().unwrap_or("-");
            let class = dev.device_class.as_deref().unwrap_or("-");

            println!(
                "{:<14} | {:<8} | {:<10} | {}",
                dev.slot, acs_status, iommu_grp, class
            );
        }
    }
}

fn print_json(output: &IommuAcsOutput) -> Result<()> {
    let json = serde_json::to_string_pretty(output)?;
    println!("{}", json);
    Ok(())
}
