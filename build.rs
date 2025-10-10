use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=manifests/");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifests_root = manifest_dir.join("manifests");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let output_file = out_dir.join("embedded_manifest_files.rs");

    // scan all manifest directories for extra files
    let embedded_files = scan_manifest_directories(&manifests_root);

    // generate rust code
    generate_embedded_files_code(&embedded_files, &output_file);
}

const MAX_FILE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// scan manifest directories and collect all non-.j2 files
fn scan_manifest_directories(manifests_root: &Path) -> HashMap<String, Vec<EmbeddedFile>> {
    let mut files_by_workload = HashMap::new();

    if !manifests_root.exists() {
        eprintln!(
            "Warning: manifests directory not found at {:?}",
            manifests_root
        );
        return files_by_workload;
    }

    eprintln!("Scanning manifests directory: {:?}", manifests_root);

    // iterate over each workload directory
    for entry in fs::read_dir(manifests_root).unwrap_or_else(|e| {
        panic!(
            "Failed to read manifests directory {:?}: {}",
            manifests_root, e
        )
    }) {
        let entry = entry.unwrap_or_else(|e| {
            panic!(
                "Failed to read directory entry in {:?}: {}",
                manifests_root, e
            )
        });
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let workload_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| panic!("Invalid workload directory name: {:?}", path))
            .to_string();

        // scan files in this workload directory
        let mut workload_files = Vec::new();
        let mut total_size = 0usize;

        for file_entry in fs::read_dir(&path)
            .unwrap_or_else(|e| panic!("Failed to read workload directory {:?}: {}", path, e))
        {
            let file_entry = file_entry
                .unwrap_or_else(|e| panic!("Failed to read file entry in {:?}: {}", path, e));
            let file_path = file_entry.path();

            if !file_path.is_file() {
                continue;
            }

            let filename = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_else(|| panic!("Invalid filename: {:?}", file_path));

            // skip .j2 template files - those are embedded directly
            if filename.ends_with(".j2") {
                continue;
            }

            // read file contents
            let contents = fs::read(&file_path)
                .unwrap_or_else(|e| panic!("Failed to read file {:?}: {}", file_path, e));

            // validate file size
            if contents.len() > MAX_FILE_SIZE {
                panic!(
                    "File {:?} is {} bytes, exceeds max size of {} bytes. \
                     Consider excluding large files from embedding.",
                    file_path,
                    contents.len(),
                    MAX_FILE_SIZE
                );
            }

            total_size += contents.len();

            // determine if file is text or binary
            let is_text = is_likely_text(&contents);

            workload_files.push(EmbeddedFile {
                filename: filename.to_string(),
                contents,
                is_text,
            });
        }

        if !workload_files.is_empty() {
            eprintln!(
                "  {}: {} files ({} bytes total)",
                workload_name,
                workload_files.len(),
                total_size
            );
            files_by_workload.insert(workload_name, workload_files);
        }
    }

    eprintln!("Embedded files for {} workloads", files_by_workload.len());
    files_by_workload
}

struct EmbeddedFile {
    filename: String,
    contents: Vec<u8>,
    is_text: bool,
}

/// simple heuristic to detect if file is text
fn is_likely_text(data: &[u8]) -> bool {
    // check first 512 bytes for null bytes and high proportion of printable chars
    let sample_size = std::cmp::min(512, data.len());
    let sample = &data[..sample_size];

    // if there are null bytes, probably binary
    if sample.contains(&0) {
        return false;
    }

    // count printable ASCII and common whitespace
    let printable_count = sample
        .iter()
        .filter(|&&b| (32..=126).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
        .count();

    // if >95% printable, consider it text
    printable_count as f64 / sample_size as f64 > 0.95
}

fn generate_embedded_files_code(
    files_by_workload: &HashMap<String, Vec<EmbeddedFile>>,
    output: &Path,
) {
    let mut code = String::new();

    code.push_str("// Auto-generated by build.rs - DO NOT EDIT\n");
    code.push_str("use std::collections::HashMap;\n");
    code.push_str("use once_cell::sync::Lazy;\n\n");

    code.push_str("/// Embedded file entry: (filename, base64_content, is_text)\n");
    code.push_str("pub type EmbeddedManifestFile = (&'static str, &'static str, bool);\n\n");

    code.push_str("/// Global registry of embedded manifest files by workload\n");
    code.push_str(
        "/// Each workload maps to a slice of (filename, base64_content, is_text) tuples\n",
    );
    code.push_str("pub static EMBEDDED_MANIFEST_FILES: Lazy<HashMap<&'static str, &'static [EmbeddedManifestFile]>> = Lazy::new(|| {\n");
    code.push_str("    let mut map = HashMap::new();\n\n");

    for (workload_name, files) in files_by_workload {
        code.push_str(&format!("    // Workload: {}\n", workload_name));

        // generate static array for this workload
        // sanitize workload name to valid rust identifier
        let array_name = format!(
            "WORKLOAD_{}",
            workload_name
                .to_uppercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );

        code.push_str(&format!(
            "    static {}: &[EmbeddedManifestFile] = &[\n",
            array_name
        ));

        for file in files {
            let filename = &file.filename;
            let is_text = file.is_text;

            // encode to base64
            let base64_content = base64_encode(&file.contents);

            code.push_str(&format!(
                "        (\"{}\", \"{}\", {}),\n",
                filename, base64_content, is_text
            ));
        }

        code.push_str("    ];\n\n");
        code.push_str(&format!(
            "    map.insert(\"{}\", {});\n\n",
            workload_name, array_name
        ));
    }

    code.push_str("    map\n");
    code.push_str("});\n\n");

    // add helper function to get files by workload
    code.push_str("/// Get embedded files for a specific workload as a slice\n");
    code.push_str("pub fn get_workload_files(workload_name: &str) -> Option<&'static [EmbeddedManifestFile]> {\n");
    code.push_str("    EMBEDDED_MANIFEST_FILES.get(workload_name).copied()\n");
    code.push_str("}\n\n");

    // add helper to get specific file
    code.push_str("/// Get a specific embedded file's base64 content by workload and filename\n");
    code.push_str(
        "pub fn get_workload_file(workload_name: &str, filename: &str) -> Option<&'static str> {\n",
    );
    code.push_str("    get_workload_files(workload_name)?\n");
    code.push_str("        .iter()\n");
    code.push_str("        .find(|(fname, _, _)| *fname == filename)\n");
    code.push_str("        .map(|(_, content, _)| *content)\n");
    code.push_str("}\n\n");

    // add helper to build configmap data hashmap
    code.push_str("/// Build a HashMap suitable for ConfigMap data field from embedded files\n");
    code.push_str("/// Keys are filenames, values are base64-encoded content\n");
    code.push_str("pub fn get_configmap_data(workload_name: &str) -> HashMap<String, String> {\n");
    code.push_str("    let mut data = HashMap::new();\n");
    code.push_str("    if let Some(files) = get_workload_files(workload_name) {\n");
    code.push_str("        for (filename, base64_content, _) in files {\n");
    code.push_str("            data.insert(filename.to_string(), base64_content.to_string());\n");
    code.push_str("        }\n");
    code.push_str("    }\n");
    code.push_str("    data\n");
    code.push_str("}\n");

    fs::write(output, code).expect("failed to write embedded files code");
}

fn base64_encode(data: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose};
    general_purpose::STANDARD.encode(data)
}
