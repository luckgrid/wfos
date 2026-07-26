//! Deterministic text and DOT renderers for validated graphs.

use super::types::GraphDocument;

/// Default max edge lines shown in human text output.
pub const TEXT_EDGE_LINE_LIMIT: usize = 10_000;

/// Render human text projection (canonical edge order already applied).
pub fn render_text(doc: &GraphDocument, freshness: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("Graph freshness: {freshness}\n"));
    out.push_str(&format!("Generated at: {}\n", doc.generated_at));
    out.push_str(&format!("Nodes: {}\n", doc.nodes.len()));
    out.push_str(&format!("Edges: {}\n", doc.edges.len()));
    out.push('\n');
    let shown = doc.edges.len().min(TEXT_EDGE_LINE_LIMIT);
    for edge in doc.edges.iter().take(shown) {
        out.push_str(&format!(
            "{} -{}-> {}\n",
            edge.from,
            edge.rel.as_str(),
            edge.to
        ));
    }
    let omitted = doc.edges.len().saturating_sub(shown);
    if omitted > 0 {
        out.push_str(&format!(
            "\n… {omitted} edge line(s) omitted (limit {TEXT_EDGE_LINE_LIMIT})\n"
        ));
    }
    out
}

/// Render Graphviz DOT from the typed graph (never from sibling graph.dot).
pub fn render_dot(doc: &GraphDocument) -> String {
    let mut out = String::from("digraph {\n");
    for node in &doc.nodes {
        out.push_str(&format!("  \"{}\";\n", escape_dot(&node.id)));
    }
    for edge in &doc.edges {
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
            escape_dot(&edge.from),
            escape_dot(&edge.to),
            escape_dot(edge.rel.as_str())
        ));
    }
    out.push_str("}\n");
    out
}

fn escape_dot(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{RegistryGeneration, SourceFingerprint};
    use crate::graph::types::{GraphEdge, GraphNode, GraphNodeKind, GraphRelation};
    use crate::graph::validate::GRAPH_UPSTREAM_PATHS;

    fn sample() -> GraphDocument {
        GraphDocument {
            generated_at: "2026-07-25T00:00:00Z".into(),
            registry_generation: RegistryGeneration {
                generated_at: "2026-07-25T00:00:00Z".into(),
                source_fingerprints: GRAPH_UPSTREAM_PATHS
                    .iter()
                    .map(|p| SourceFingerprint {
                        path: (*p).into(),
                        algorithm: "sha256".into(),
                        digest: "ab".repeat(32),
                    })
                    .collect(),
            },
            nodes: vec![
                GraphNode {
                    id: r#"a"b\c"#.into(),
                    kind: GraphNodeKind::Package,
                },
                GraphNode {
                    id: "isolated".into(),
                    kind: GraphNodeKind::Package,
                },
            ],
            edges: vec![GraphEdge {
                from: r#"a"b\c"#.into(),
                rel: GraphRelation::Uses,
                to: "isolated".into(),
            }],
        }
    }

    #[test]
    fn text_is_stable() {
        let doc = sample();
        let a = render_text(&doc, "hit");
        let b = render_text(&doc, "hit");
        assert_eq!(a, b);
        assert!(a.contains("Graph freshness: hit"));
        assert!(a.contains(r#"a"b\c -uses-> isolated"#));
    }

    #[test]
    fn dot_escapes_and_includes_isolated() {
        let doc = sample();
        let dot = render_dot(&doc);
        assert!(dot.contains(r#"a\"b\\c"#));
        assert!(dot.contains("\"isolated\""));
    }
}
