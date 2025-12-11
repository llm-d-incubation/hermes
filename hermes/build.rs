use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../charts/");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let charts_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .join("charts");

    generate_embedded_charts(&out_dir, &charts_dir);
    generate_empty_embedded_files(&out_dir);
    generate_empty_common_templates(&out_dir);
}

fn generate_embedded_charts(out_dir: &Path, charts_dir: &Path) {
    let output_file = out_dir.join("embedded_charts.rs");
    let mut code = String::from(
        r#"// auto-generated: embedded Helm charts

/// (relative_path, base64_content)
pub type EmbeddedChartFile = (&'static str, &'static str);

/// get all files for a chart, returns vec of (relative_path, base64_content)
pub fn get_chart_files(chart_name: &str) -> Option<&'static [EmbeddedChartFile]> {
    match chart_name {
"#,
    );

    let mut charts: Vec<String> = Vec::new();

    if charts_dir.exists() {
        for entry in fs::read_dir(charts_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                let chart_name = path.file_name().unwrap().to_str().unwrap();
                if path.join("Chart.yaml").exists() {
                    charts.push(chart_name.to_string());
                }
            }
        }
    }

    charts.sort();

    for chart_name in &charts {
        code.push_str(&format!(
            "        \"{}\" => Some(CHART_{}_FILES),\n",
            chart_name,
            chart_name.to_uppercase().replace('-', "_")
        ));
    }

    code.push_str(
        r#"        _ => None,
    }
}

/// list all available chart names
pub fn list_charts() -> &'static [&'static str] {
    &[
"#,
    );

    for chart_name in &charts {
        code.push_str(&format!("        \"{}\",\n", chart_name));
    }

    code.push_str(
        r#"    ]
}

"#,
    );

    // generate file arrays for each chart
    for chart_name in &charts {
        let chart_path = charts_dir.join(chart_name);
        let files = collect_chart_files(&chart_path, &chart_path);

        code.push_str(&format!(
            "static CHART_{}_FILES: &[EmbeddedChartFile] = &[\n",
            chart_name.to_uppercase().replace('-', "_")
        ));

        for (rel_path, content) in files {
            let encoded = BASE64.encode(&content);
            code.push_str(&format!("    (\"{}\", \"{}\"),\n", rel_path, encoded));
        }

        code.push_str("];\n\n");
    }

    fs::write(output_file, code).expect("failed to write embedded charts");
}

fn collect_chart_files(base_path: &Path, current_path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();

    if current_path.is_dir() {
        for entry in fs::read_dir(current_path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_chart_files(base_path, &path));
            } else {
                let rel_path = path.strip_prefix(base_path).unwrap();
                let content = fs::read(&path).unwrap();
                files.push((rel_path.to_string_lossy().to_string(), content));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn generate_empty_embedded_files(out_dir: &Path) {
    let output_file = out_dir.join("embedded_manifest_files.rs");
    let code = r#"// auto-generated stub - manifests are now Helm charts in charts/
use std::collections::HashMap;

/// placeholder type for backwards compatibility
pub type EmbeddedManifestFile = (&'static str, &'static str, bool);

/// returns empty - files are now in Helm charts
pub fn get_workload_files(_workload_name: &str) -> Option<&'static [EmbeddedManifestFile]> {
    None
}

/// returns empty - files are now in Helm charts
pub fn get_workload_file(_workload_name: &str, _filename: &str) -> Option<&'static str> {
    None
}

/// returns empty map - files are now in Helm charts
pub fn get_configmap_data(_workload_name: &str) -> HashMap<String, String> {
    HashMap::new()
}
"#;
    fs::write(output_file, code).expect("failed to write embedded files stub");
}

fn generate_empty_common_templates(out_dir: &Path) {
    let output_file = out_dir.join("common_templates.rs");
    let code = r#"// auto-generated stub - templates are now in Helm charts
pub fn load_common_templates(_env: &mut minijinja::Environment) {
    // no-op: templates are now in Helm charts
}
"#;
    fs::write(output_file, code).expect("failed to write common templates stub");
}
