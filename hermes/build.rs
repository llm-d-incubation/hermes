use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // charts are now in charts/ directory, no longer embedding manifests
    println!("cargo:rerun-if-changed=../charts/");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // generate empty stubs for backwards compatibility during transition
    generate_empty_embedded_files(&out_dir);
    generate_empty_common_templates(&out_dir);
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
