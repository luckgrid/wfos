//! E09.S7 Phase 2 — typed graph projection (zero-spawn).

pub mod render;
pub mod types;
pub mod validate;

pub use render::{TEXT_EDGE_LINE_LIMIT, render_dot, render_text};
pub use types::{
    GRAPH_EDGE_LIMIT, GRAPH_FILE_LIMIT_BYTES, GRAPH_ID_LIMIT_BYTES, GRAPH_NODE_LIMIT,
    GraphDocument, GraphEdge, GraphNode, GraphNodeKind, GraphRelation,
};
pub use validate::{
    GraphLoadOutcome, canonicalize_graph, evaluate_graph_freshness, validate_graph_document,
};
