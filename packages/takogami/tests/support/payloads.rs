//! Plan-aligned bin machine payloads (S7-R03). No invented schema_version/kind/mutation.

use serde_json::{Value, json};

pub fn sample_inventory(root: &str) -> Value {
    json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "root": root,
        "summary": {
            "total": 0,
            "with_manifest": 0
        },
        "workflows": []
    })
}

pub fn sample_cleanup_plan(mode: &str) -> Value {
    json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "mode": mode,
        "scope": null,
        "inventory_generated_at": "2026-07-25T00:00:00Z",
        "inventory_refreshed": false,
        "summary": {
            "total": 0,
            "advisory": 0,
            "would_archive": 0,
            "would_delete": 0,
            "blocked": 0
        },
        "entries": [],
        "mutation_executed": false
    })
}

/// Invalid fixture: every other field consistent; only mutation_executed is wrong.
pub fn sample_cleanup_mutation_true() -> Value {
    json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "mode": "report-only",
        "scope": null,
        "inventory_generated_at": "2026-07-25T00:00:00Z",
        "inventory_refreshed": false,
        "summary": {
            "total": 1,
            "advisory": 0,
            "would_archive": 0,
            "would_delete": 0,
            "blocked": 1
        },
        "entries": [{
            "path": "Build/bin/demo",
            "disposition": "blocked",
            "reason": "no-manifest",
            "retention": null,
            "approved_to_matches": null
        }],
        "mutation_executed": true
    })
}
