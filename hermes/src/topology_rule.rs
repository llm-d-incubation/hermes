use anyhow::{Context as AnyhowContext, Result, anyhow};
use cel::{Context, FunctionContext, Program};
use k8s_openapi::api::core::v1::Node;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::models::{TopologyDetection, TopologyType};

/// Custom CEL function to extract a substring using regex
/// Usage: extract("pokprod-b93r43s0", "r(\\d+)")  -> "43"
fn extract_regex(
    ftx: &FunctionContext,
    input: Arc<String>,
    pattern: Arc<String>,
) -> cel::ResolveResult {
    match Regex::new(&pattern) {
        Ok(re) => {
            if let Some(captures) = re.captures(&input) {
                // return first capture group if it exists, otherwise the whole match
                let result = if captures.len() > 1 {
                    captures.get(1).map(|m| m.as_str()).unwrap_or("")
                } else {
                    captures.get(0).map(|m| m.as_str()).unwrap_or("")
                };
                Ok(cel::Value::String(Arc::new(result.to_string())))
            } else {
                Ok(cel::Value::String(Arc::new(String::new())))
            }
        }
        Err(err) => ftx.error(format!("Invalid regex pattern: {}", err)).into(),
    }
}

/// Evaluates a CEL-based topology extraction rule
pub fn evaluate_topology_rule(
    node: &Node,
    labels: &BTreeMap<String, String>,
    rule: &str,
) -> Result<Option<String>> {
    let node_name = node
        .metadata
        .name
        .as_ref()
        .ok_or_else(|| anyhow!("Node has no name"))?;

    // compile the CEL program
    let program = Program::compile(rule).context("Failed to compile CEL topology rule")?;

    // create evaluation context with node data
    let mut context = Context::default();

    // add custom functions
    context.add_function("extract", extract_regex);

    // add node name
    context
        .add_variable("node_name", node_name.to_string())
        .context("Failed to add node_name to CEL context")?;

    // add labels as a map
    let labels_map: std::collections::HashMap<String, String> =
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    context
        .add_variable("node_labels", labels_map)
        .context("Failed to add node_labels to CEL context")?;

    // evaluate the expression
    let result = program
        .execute(&context)
        .context("Failed to execute CEL topology rule")?;

    // convert result to string if possible
    match result {
        cel::Value::String(s) => {
            if s.is_empty() {
                Ok(None)
            } else {
                Ok(Some(s.to_string()))
            }
        }
        cel::Value::Int(i) => Ok(Some(i.to_string())),
        cel::Value::UInt(u) => Ok(Some(u.to_string())),
        cel::Value::Null => Ok(None),
        _ => Ok(Some(format!("{:?}", result))),
    }
}

/// Create a TopologyDetection for a custom rule
pub fn create_custom_topology_detection(rule: &str) -> TopologyDetection {
    TopologyDetection {
        topology_type: TopologyType::Custom,
        detection_method: format!("Custom CEL rule: {}", rule),
        confidence: "High".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn test_extract_rack_and_get_tens_digit() {
        let labels = BTreeMap::new();

        // test node in rack 43 (should be in topology block "4")
        let node1 = Node {
            metadata: ObjectMeta {
                name: Some("pokprod-b93r43s0".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        // extract rack number (43) and get tens digit (4) for topology grouping
        let rule = r#"string(int(extract(node_name, "r(\\d+)")) / 10)"#;
        let result = evaluate_topology_rule(&node1, &labels, rule).unwrap();
        assert_eq!(result, Some("4".to_string()));

        // test node in rack 52 (should be in topology block "5")
        let node2 = Node {
            metadata: ObjectMeta {
                name: Some("pokprod-b93r52s1".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = evaluate_topology_rule(&node2, &labels, rule).unwrap();
        assert_eq!(result, Some("5".to_string()));

        // test node in rack 9 (should be in topology block "0")
        let node3 = Node {
            metadata: ObjectMeta {
                name: Some("pokprod-b93r9s0".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = evaluate_topology_rule(&node3, &labels, rule).unwrap();
        assert_eq!(result, Some("0".to_string()));
    }

    #[test]
    fn test_extract_basic() {
        let labels = BTreeMap::new();
        let node = Node {
            metadata: ObjectMeta {
                name: Some("pokprod-b93r43s0".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        // test basic extraction
        let rule = r#"extract(node_name, "r(\\d+)")"#;
        let result = evaluate_topology_rule(&node, &labels, rule).unwrap();
        assert_eq!(result, Some("43".to_string()));
    }

    #[test]
    fn test_combine_with_label() {
        let mut labels = BTreeMap::new();
        labels.insert("zone".to_string(), "us-east-1a".to_string());

        let node = Node {
            metadata: ObjectMeta {
                name: Some("node-123".to_string()),
                labels: Some(labels.clone()),
                ..Default::default()
            },
            ..Default::default()
        };

        // combine extracted value with label
        let rule = r#"extract(node_name, "(node)-\\d+") + "-" + node_labels["zone"]"#;
        let result = evaluate_topology_rule(&node, &labels, rule).unwrap();
        assert_eq!(result, Some("node-us-east-1a".to_string()));
    }
}
