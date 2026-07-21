//! Command handlers for discovery / query / doctor (E09.S3).

use std::path::Path;

use crate::cli::{Command, ListTarget};
use crate::doctor::{self, DoctorInputs};
use crate::error::ControllerError;
use crate::output::OutputSink;
use crate::registry::{
    ExternalAdapters, Freshness, ProcessAdapters, RefreshKind, RegistryAccess, discover_from_scan,
    filter_tools, filter_units, find_unit, parse_filters, resolve_registry_paths,
};

pub fn dispatch_implemented(
    command: &Command,
    sink: &OutputSink,
    cli_state_home: Option<&Path>,
) -> Result<u8, ControllerError> {
    match command {
        Command::Doctor => run_doctor(sink, cli_state_home),
        Command::Scan { refresh } => run_scan(sink, *refresh),
        Command::List { target, filters } => run_list(sink, target, filters),
        Command::Info { unit } => run_info(sink, unit),
        Command::Tools => run_tools(sink),
        Command::Interfaces { validate } => run_interfaces(sink, *validate),
        _ => Err(ControllerError::internal(
            "dispatch_implemented called for unimplemented command",
        )),
    }
}

fn access() -> Result<RegistryAccess, ControllerError> {
    Ok(RegistryAccess::new(resolve_registry_paths()?))
}

fn run_doctor(sink: &OutputSink, cli_state_home: Option<&Path>) -> Result<u8, ControllerError> {
    let reg = access().ok();
    let report = doctor::run_doctor(DoctorInputs {
        registry: reg.as_ref(),
        cli_state_home,
        path_var: None,
    });
    sink.emit_doctor(&report)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_scan(sink: &OutputSink, refresh: bool) -> Result<u8, ControllerError> {
    let access = access()?;
    if refresh {
        let adapters = ProcessAdapters;
        let cwd = access
            .paths
            .registry_root
            .ancestors()
            .nth(3) // packages/ontarch/registry → wfos
            .unwrap_or(Path::new("."));
        let out = adapters.refresh(RefreshKind::Scan, cwd)?;
        if !out.status.success() {
            return Err(ControllerError::unavailable_source(format!(
                "ontarch scan refresh failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
    }
    let (scan, scan_fresh) = access.load_scan()?;
    let (units, units_fresh) = access.load_units()?;
    let freshness = match (scan_fresh, units_fresh) {
        (Freshness::Miss, Freshness::Miss) => Freshness::Miss,
        (Freshness::Hit, Freshness::Hit) => Freshness::Hit,
        _ => Freshness::Stale,
    };
    let mut units_doc = units;
    if matches!(freshness, Freshness::Miss) {
        units_doc.units = access.source_fallback_units()?;
    }
    let discovery = discover_from_scan(&scan, &units_doc, freshness)?;
    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "workspaces": discovery.workspaces,
        "units": discovery.units,
        "provisional": discovery.provisional,
        "lint_check_commands_evidence_only": true,
    });
    let human = vec![
        format!("takogami scan (freshness: {})", freshness.as_str()),
        format!("  descriptor-backed units: {}", discovery.units.len()),
        format!(
            "  provisional (descriptor-less): {}",
            discovery.provisional.len()
        ),
        format!("  workspaces: {}", discovery.workspaces.len()),
        "  note: lint_check_commands are evidence only (not executed)".into(),
    ];
    sink.emit_success("scan", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_list(
    sink: &OutputSink,
    target: &ListTarget,
    filters: &[String],
) -> Result<u8, ControllerError> {
    let access = access()?;
    let parsed = parse_filters(filters)?;
    match target {
        ListTarget::Units => {
            let (mut doc, freshness) = access.load_units()?;
            if freshness == Freshness::Miss {
                doc.units = access.source_fallback_units()?;
            }
            let units = filter_units(&doc.units, &parsed);
            let data = serde_json::json!({
                "freshness": freshness.as_str(),
                "count": units.len(),
                "units": units,
            });
            let mut human = vec![format!(
                "takogami list units (freshness: {}, count: {})",
                freshness.as_str(),
                units.len()
            )];
            for u in &units {
                human.push(format!(
                    "  {}  {}  {}",
                    u.id,
                    u.kind.as_deref().unwrap_or("-"),
                    u.path.as_deref().unwrap_or("-")
                ));
            }
            sink.emit_success("list", data, Some(freshness), &human)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
        ListTarget::Tools => {
            let (doc, freshness) = access.load_tools()?;
            let tools = filter_tools(&doc.tools, &parsed);
            let data = serde_json::json!({
                "freshness": freshness.as_str(),
                "count": tools.len(),
                "tools": tools,
                "source": "ontarch/tools.json (panoply projection)",
            });
            let mut human = vec![format!("takogami list tools (count: {})", tools.len())];
            for t in &tools {
                human.push(format!(
                    "  {}  module={}  installed={}",
                    t.id,
                    t.module.as_deref().unwrap_or("-"),
                    t.installed
                        .map(|b| if b { "true" } else { "false" })
                        .unwrap_or("-")
                ));
            }
            sink.emit_success("list", data, Some(freshness), &human)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
    }
}

fn run_info(sink: &OutputSink, unit_id: &str) -> Result<u8, ControllerError> {
    let access = access()?;
    let (mut doc, freshness) = access.load_units()?;
    if freshness == Freshness::Miss {
        doc.units = access.source_fallback_units()?;
    }
    let unit = find_unit(&doc.units, unit_id)?.clone();
    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "unit": unit,
        "provenance": {
            "source": unit.source,
            "path": unit.path,
            "provisional": unit.provisional,
            "routing_complete": unit.routing_complete,
        }
    });
    let human = vec![
        format!(
            "takogami info {unit_id} (freshness: {})",
            freshness.as_str()
        ),
        format!("  kind: {}", unit.kind.as_deref().unwrap_or("-")),
        format!("  path: {}", unit.path.as_deref().unwrap_or("-")),
        format!("  source: {}", unit.source.as_deref().unwrap_or("-")),
        format!("  provisional: {}", unit.provisional),
    ];
    sink.emit_success("info", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_tools(sink: &OutputSink) -> Result<u8, ControllerError> {
    let access = access()?;
    let (doc, freshness) = access.load_tools()?;
    let adapters = ProcessAdapters;
    let panoply: Option<serde_json::Value> = adapters.panoply_doctor_json().ok().and_then(|o| {
        if o.status.success() {
            serde_json::from_slice(&o.stdout).ok()
        } else {
            None
        }
    });

    let classified: Vec<_> = doc
        .tools
        .iter()
        .map(|t| {
            let class = if t.default == Some(true) {
                "required"
            } else if matches!(t.id.as_str(), "herdr" | "tmux" | "rtk") {
                "optional"
            } else if t.installed == Some(true) {
                "selected"
            } else {
                "optional"
            };
            serde_json::json!({
                "id": t.id,
                "module": t.module,
                "installed": t.installed,
                "default": t.default,
                "capability_class": class,
                "version": t.version,
            })
        })
        .collect();

    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "tools": classified,
        "panoply_doctor": panoply,
        "notes": [
            "Tools are projected from Panoply/Ontarch — Takogami does not maintain a second catalog.",
            "Absence of Herdr is never a required failure for base doctor.",
        ],
    });
    let human = vec![
        format!("takogami tools ({} projected)", classified.len()),
        "  source: Ontarch tools.json + optional panoply doctor --json".into(),
    ];
    sink.emit_success("tools", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_interfaces(sink: &OutputSink, validate: bool) -> Result<u8, ControllerError> {
    let access = access()?;
    let (readable, detail) = access.contracts_readable();
    if validate {
        let adapters = ProcessAdapters;
        let cwd = access
            .paths
            .workspace_root
            .join("Build/src/workspaces/wfos");
        let cwd = if cwd.is_dir() {
            cwd
        } else {
            access.paths.registry_root.clone()
        };
        let out = adapters.validate(&cwd)?;
        let ok = out.status.success();
        let data = serde_json::json!({
            "validate": true,
            "ok": ok,
            "contracts_readable": readable,
            "detail": detail,
            "stdout": String::from_utf8_lossy(&out.stdout),
            "stderr": String::from_utf8_lossy(&out.stderr),
        });
        if sink.json {
            let mut env = crate::contracts::CommandEnvelope::ok("interfaces", Some(data));
            if !ok {
                env.status = "error".into();
                env.exit_code = crate::exit_codes::CONTRACT;
            }
            sink.emit_envelope(&env)
                .map_err(|e| ControllerError::internal(e.to_string()))?;
            Ok(if ok {
                crate::exit_codes::SUCCESS
            } else {
                crate::exit_codes::CONTRACT
            })
        } else {
            writeln_human(&format!(
                "takogami interfaces --validate: {}",
                if ok { "PASS" } else { "FAIL" }
            ))?;
            Ok(if ok {
                crate::exit_codes::SUCCESS
            } else {
                crate::exit_codes::CONTRACT
            })
        }
    } else {
        let data = serde_json::json!({
            "validate": false,
            "contracts_readable": readable,
            "detail": detail,
        });
        let human = vec![
            "takogami interfaces (readability only; pass --validate to run ontarch validate)"
                .into(),
            format!("  contracts_readable: {readable}"),
            format!("  {detail}"),
        ];
        sink.emit_success("interfaces", data, None, &human)
            .map_err(|e| ControllerError::internal(e.to_string()))
    }
}

fn writeln_human(line: &str) -> Result<(), ControllerError> {
    use std::io::Write;
    writeln!(std::io::stdout(), "{line}").map_err(|e| ControllerError::internal(e.to_string()))
}
