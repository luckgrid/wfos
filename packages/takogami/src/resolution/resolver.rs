//! Ordered deterministic resolution pipeline.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::contracts::{RegistryGeneration, ResolvedCommand, SCHEMA_VERSION, fingerprint_file};
use crate::error::ControllerError;
use crate::registry::{
    AuthoredUnitDescriptor, Freshness, RegistryAccess, ToolRecord, UnitDefinition, UnitRecord,
};

use super::entrypoint::{NormalizedEntrypoint, normalize_entrypoint};
use super::executable::{ExecutableLocator, FilesystemLocator};
use super::explain::{
    FreshnessExplanation, PartialRequestView, PartialResolutionTrace, PartialUnitView,
    ResolutionExplanation, ResolutionStep, SafeEntrypointView, SafeSourceView,
    explanation_from_plan,
};
use super::paths::{resolve_cwd, resolve_manifests, workspace_relative_display};
use super::plan::{Actor, PolicyEvaluationInput, SealedExecutionPlan};
use super::profile::{SelectedProfile, collect_policy_refs, select_profile};
use super::request::{CorrelationIdGenerator, ResolutionRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    NativeDirect,
    PanoplyTool,
    MoonTask,
}

impl BackendKind {
    pub fn wire_labels(self) -> (&'static str, &'static str) {
        match self {
            Self::NativeDirect => ("native", "direct"),
            Self::PanoplyTool => ("panoply", "native-tool"),
            Self::MoonTask => ("moon", "moon-task"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResolutionCode {
    RegistryUnavailable { message: String },
    UnitNotFound { id: String },
    UnitAmbiguous { id: String, candidates: Vec<String> },
    RoutingIncomplete { id: String },
    DescriptorUnavailable { message: String },
    DescriptorAmbiguous { id: String, candidates: Vec<String> },
    InvalidDescriptor { message: String },
    MissingEntrypoint { verb: String },
    UnsafeLegacyEntrypoint { message: String },
    InvalidCwd { message: String },
    MissingManifest { message: String },
    ManifestAmbiguous { candidates: Vec<String> },
    UnsupportedBackend { message: String },
    MissingExecutable { message: String },
    ExecutableAmbiguous { candidates: Vec<String> },
    ProfileRequired,
    ProfileNotFound { id: String },
    ProfileAmbiguous { id: String },
    PolicyNotFound { id: String },
    PolicyDuplicate { id: String },
}

impl ResolutionCode {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RegistryUnavailable { .. } => "registry_unavailable",
            Self::UnitNotFound { .. } => "unit_not_found",
            Self::UnitAmbiguous { .. } => "unit_ambiguous",
            Self::RoutingIncomplete { .. } => "routing_incomplete",
            Self::DescriptorUnavailable { .. } => "descriptor_unavailable",
            Self::DescriptorAmbiguous { .. } => "descriptor_ambiguous",
            Self::InvalidDescriptor { .. } => "invalid_descriptor",
            Self::MissingEntrypoint { .. } => "missing_entrypoint",
            Self::UnsafeLegacyEntrypoint { .. } => "unsafe_legacy_entrypoint",
            Self::InvalidCwd { .. } => "invalid_cwd",
            Self::MissingManifest { .. } => "missing_manifest",
            Self::ManifestAmbiguous { .. } => "manifest_ambiguous",
            Self::UnsupportedBackend { .. } => "unsupported_backend",
            Self::MissingExecutable { .. } => "missing_executable",
            Self::ExecutableAmbiguous { .. } => "executable_ambiguous",
            Self::ProfileRequired => "profile_required",
            Self::ProfileNotFound { .. } => "profile_not_found",
            Self::ProfileAmbiguous { .. } => "profile_ambiguous",
            Self::PolicyNotFound { .. } => "policy_not_found",
            Self::PolicyDuplicate { .. } => "policy_duplicate",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::RegistryUnavailable { message }
            | Self::DescriptorUnavailable { message }
            | Self::InvalidDescriptor { message }
            | Self::UnsafeLegacyEntrypoint { message }
            | Self::InvalidCwd { message }
            | Self::MissingManifest { message }
            | Self::UnsupportedBackend { message }
            | Self::MissingExecutable { message } => message.clone(),
            Self::UnitNotFound { id } => format!("unit `{id}` not found"),
            Self::UnitAmbiguous { id, candidates } => {
                format!("unit `{id}` ambiguous: {}", candidates.join(", "))
            }
            Self::RoutingIncomplete { id } => {
                format!("unit `{id}` has incomplete routing metadata")
            }
            Self::DescriptorAmbiguous { id, candidates } => {
                format!("descriptor for `{id}` ambiguous: {}", candidates.join(", "))
            }
            Self::MissingEntrypoint { verb } => format!("missing entrypoint for verb `{verb}`"),
            Self::ManifestAmbiguous { candidates } => {
                format!("manifest ambiguous: {}", candidates.join(", "))
            }
            Self::ExecutableAmbiguous { candidates } => {
                format!("executable ambiguous: {}", candidates.join(", "))
            }
            Self::ProfileRequired => {
                "profile required (pass --profile, set TAKOGAMI_PROFILE, or define workspace-dev)"
                    .into()
            }
            Self::ProfileNotFound { id } => format!("profile `{id}` not found"),
            Self::ProfileAmbiguous { id } => format!("profile `{id}` ambiguous"),
            Self::PolicyNotFound { id } => format!("policy `{id}` not found"),
            Self::PolicyDuplicate { id } => format!("duplicate policy id `{id}`"),
        }
    }

    pub fn into_error(mut self, explanation_partial: PartialResolutionTrace) -> ControllerError {
        match &mut self {
            Self::UnitAmbiguous { candidates, .. }
            | Self::DescriptorAmbiguous { candidates, .. }
            | Self::ManifestAmbiguous { candidates }
            | Self::ExecutableAmbiguous { candidates } => {
                candidates.sort();
                candidates.dedup();
            }
            _ => {}
        }
        ControllerError::Resolution {
            code: self.code().into(),
            message: self.message(),
            session_id: Some(explanation_partial.session_id.clone()),
            explanation_partial: serde_json::to_value(explanation_partial).ok().map(Box::new),
        }
    }
}

pub struct ResolverInputs<'a> {
    pub access: &'a RegistryAccess,
    pub path_dirs: Vec<PathBuf>,
    pub env_profile: Option<String>,
    pub locator: &'a dyn ExecutableLocator,
    pub id_gen: &'a mut dyn CorrelationIdGenerator,
}

pub struct Resolver<'a> {
    inputs: ResolverInputs<'a>,
}

#[derive(Debug)]
pub struct ResolveSuccess {
    pub plan: SealedExecutionPlan,
    pub explanation: ResolutionExplanation,
    pub selected: SelectedProfile,
    pub freshness: Freshness,
    pub explain_requested: bool,
    pub execute_requested: bool,
    pub policy_root: PathBuf,
}

impl ResolveSuccess {
    /// Immutable S5 handoff. Policy evaluation must not re-resolve any plan input.
    pub fn policy_evaluation_input(&self) -> PolicyEvaluationInput {
        let resolved = self.plan.resolved();
        PolicyEvaluationInput::new(
            Actor::Agent,
            super::plan::RequestedOperation::from_resolution(
                &resolved.unit_id,
                &resolved.verb,
                self.explain_requested,
                self.execute_requested,
            ),
            self.plan.clone(),
            self.selected.profile.clone(),
            self.selected.policies.clone(),
            self.selected.policy_origins.clone(),
            self.policy_root.clone(),
        )
    }
}

impl<'a> Resolver<'a> {
    pub fn new(inputs: ResolverInputs<'a>) -> Self {
        Self { inputs }
    }

    pub fn resolve(
        &mut self,
        mut request: ResolutionRequest,
    ) -> Result<ResolveSuccess, ControllerError> {
        if request.session_id.is_empty() {
            request.session_id = self.inputs.id_gen.next_id();
        }
        let mut trace = PartialResolutionTrace {
            session_id: request.session_id.clone(),
            mode: "plan_only".into(),
            request: PartialRequestView {
                unit_id: request.unit_id.clone(),
                verb: request.verb.as_str().into(),
                requested_profile: request.explicit_profile.clone(),
            },
            completed_steps: vec![ResolutionStep::CorrelationId],
            freshness: None,
            unit: None,
            descriptor: None,
            entrypoint: None,
            manifests: Vec::new(),
            executable: None,
            profile_id: None,
            policy_ids: Vec::new(),
        };
        self.resolve_inner(request, &mut trace)
            .map_err(|code| code.into_error(trace))
    }

    fn resolve_inner(
        &mut self,
        request: ResolutionRequest,
        trace: &mut PartialResolutionTrace,
    ) -> Result<ResolveSuccess, ResolutionCode> {
        // Step 2 — registry roots already on access
        let workspace_root = self.inputs.access.paths.workspace_root.clone();
        if !self.inputs.access.paths.registry_root.is_dir() {
            return Err(ResolutionCode::RegistryUnavailable {
                message: "configured registry root is missing".into(),
            });
        }
        trace.completed_steps.push(ResolutionStep::Registry);

        // Step 3 — unit
        let (units_doc, freshness) =
            self.inputs
                .access
                .load_units()
                .map_err(|_| ResolutionCode::RegistryUnavailable {
                    message: "units registry is unavailable or invalid".into(),
                })?;
        let mut units = units_doc.units.clone();
        if freshness == Freshness::Miss {
            units = self.inputs.access.source_fallback_units().map_err(|_| {
                ResolutionCode::RegistryUnavailable {
                    message: "authored unit sources are unavailable".into(),
                }
            })?;
        }
        trace.freshness = Some(FreshnessExplanation {
            registry_cache: freshness.as_str().into(),
        });

        let unit_rec = match select_unit(&units, &request.unit_id) {
            Ok(u) => u.clone(),
            Err(ResolutionCode::UnitNotFound { .. })
                if matches!(freshness, Freshness::Miss | Freshness::Stale) =>
            {
                // Miss/stale may still resolve solely from authored descriptors.
                UnitRecord {
                    id: request.unit_id.clone(),
                    kind: None,
                    title: None,
                    status: None,
                    domain: None,
                    layer: None,
                    stack: None,
                    owner: None,
                    runtime: None,
                    path: None,
                    native_manifests: vec![],
                    entrypoints: Default::default(),
                    cli: None,
                    provides: vec![],
                    requires: vec![],
                    policy: None,
                    source: None,
                    provisional: true,
                    routing_complete: Some(false),
                }
            }
            Err(e) => return Err(e),
        };

        // Hit projections that are provisional must fail closed; miss/stale recover via authored.
        if matches!(freshness, Freshness::Hit)
            && (unit_rec.provisional || unit_rec.routing_complete == Some(false))
        {
            return Err(ResolutionCode::RoutingIncomplete {
                id: request.unit_id.clone(),
            });
        }
        trace.unit = Some(PartialUnitView {
            id: unit_rec.id.clone(),
        });
        trace.completed_steps.push(ResolutionStep::Unit);

        // Step 4 — authoritative descriptor
        let unit_def = load_unit_definition(self.inputs.access, &unit_rec, freshness)?;
        trace.descriptor = Some(SafeSourceView {
            descriptor: safe_workspace_path(&workspace_root, Path::new(&unit_def.descriptor_path)),
        });
        trace.completed_steps.push(ResolutionStep::Descriptor);

        // Step 5 — entrypoint
        let verb_key = request.verb.as_str();
        let entry_def = unit_def.entrypoints.get(verb_key).ok_or_else(|| {
            ResolutionCode::MissingEntrypoint {
                verb: verb_key.into(),
            }
        })?;
        let mut entry = normalize_entrypoint(entry_def)?;
        trace.entrypoint = Some(SafeEntrypointView {
            program: entry.program.clone(),
            execution_class: entry.execution_class.as_str().into(),
            runtime_provider: entry.runtime_provider.clone(),
        });
        trace.completed_steps.push(ResolutionStep::Entrypoint);

        // Step 8 — tools + classify + executable
        let (tools_doc, _) =
            self.inputs
                .access
                .load_tools()
                .map_err(|_| ResolutionCode::RegistryUnavailable {
                    message: "tool registry is unavailable or invalid".into(),
                })?;
        let backend = classify_backend(&entry, &tools_doc.tools)?;
        let (backend_label, adapter_label) = authored_or_wire_labels(&entry, backend);

        // Step 6 — cwd
        let cwd = resolve_cwd(&workspace_root, &unit_def, &entry)?;
        trace.completed_steps.push(ResolutionStep::Cwd);

        // Step 7 — manifests
        let (manifest_display, manifest_paths) =
            resolve_manifests(&workspace_root, &cwd.canonical, &unit_def, &entry, backend)?;
        trace.manifests = manifest_display.clone();
        trace.completed_steps.push(ResolutionStep::Manifests);

        validate_backend_contract(&entry, backend, &tools_doc.tools)?;
        trace.completed_steps.push(ResolutionStep::Backend);
        let executable = self.inputs.locator.locate(
            &entry.program,
            &cwd.canonical,
            &self.inputs.path_dirs,
            backend,
            &tools_doc.tools,
            &workspace_root,
        )?;
        trace.executable = Some(executable.provenance.clone());
        trace.completed_steps.push(ResolutionStep::Executable);

        // Step 9 — profile + policies
        let profiles = self.inputs.access.load_profiles().map_err(|_| {
            ResolutionCode::RegistryUnavailable {
                message: "profile registry is unavailable or invalid".into(),
            }
        })?;
        let profile = select_profile(
            &profiles,
            request.explicit_profile.as_deref(),
            self.inputs.env_profile.as_deref(),
        )?;
        trace.profile_id = Some(profile.id.clone());
        trace.completed_steps.push(ResolutionStep::Profile);
        let policies = self.inputs.access.load_policies().map_err(|_| {
            ResolutionCode::RegistryUnavailable {
                message: "policy registry is unavailable or invalid".into(),
            }
        })?;
        let selected = collect_policy_refs(&policies, &profile, &entry, &request.unit_id)?;
        trace.policy_ids = selected.policy_ids.clone();
        trace.completed_steps.push(ResolutionStep::Policies);

        // Stable env keys
        let mut env_keys = entry.env_keys.clone();
        env_keys.sort();
        env_keys.dedup();
        entry.env_keys = env_keys;

        let descriptor_path = workspace_relative_display(
            &workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.clone()),
            Path::new(&unit_def.descriptor_path),
        );
        let fp = fingerprint_file(Path::new(&unit_def.descriptor_path), &descriptor_path)
            .or_else(|_| {
                let abs = workspace_root.join(&unit_def.descriptor_path);
                fingerprint_file(&abs, &descriptor_path)
            })
            .map_err(|_| ResolutionCode::DescriptorUnavailable {
                message: "cannot fingerprint selected descriptor".into(),
            })?;

        let mut diagnostics = entry.diagnostics.clone();
        diagnostics.sort_by(|a, b| a.code.cmp(&b.code).then(a.message.cmp(&b.message)));

        let resolved = ResolvedCommand {
            schema_version: SCHEMA_VERSION.into(),
            session_id: request.session_id.clone(),
            unit_id: request.unit_id.clone(),
            verb: verb_key.into(),
            descriptor_path,
            descriptor_fingerprint: format!("sha256:{}", fp.digest),
            native_manifests: manifest_display,
            backend: backend_label,
            adapter: adapter_label,
            program: entry.program.clone(),
            argv: entry.args.clone(),
            cwd: cwd.display.clone(),
            env_keys: entry.env_keys.clone(),
            profile_id: selected.profile.id.clone(),
            policy_ids: selected.policy_ids.clone(),
            registry_generation: units_doc.registry_generation.unwrap_or(RegistryGeneration {
                generated_at: units_doc.generated_at.clone(),
                source_fingerprints: vec![fp],
            }),
            execution_class: entry.execution_class,
            runtime_provider: entry.runtime_provider.clone(),
        };

        let plan = SealedExecutionPlan::seal(
            resolved,
            executable.canonical,
            cwd.canonical,
            manifest_paths,
            executable.provenance,
            diagnostics,
        );
        trace.completed_steps.push(ResolutionStep::Plan);

        let mut explanation = explanation_from_plan(&plan, &selected, freshness);
        explanation.completed_steps = trace.completed_steps.clone();

        let workspace = &self.inputs.access.paths.workspace_root;
        let policy_root = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.clone());

        Ok(ResolveSuccess {
            plan,
            explanation,
            selected,
            freshness,
            explain_requested: request.explain,
            execute_requested: request.execute_requested,
            policy_root,
        })
    }
}

pub fn resolve(
    access: &RegistryAccess,
    request: ResolutionRequest,
    path_dirs: Vec<PathBuf>,
    env_profile: Option<String>,
    id_gen: &mut dyn CorrelationIdGenerator,
) -> Result<ResolveSuccess, ControllerError> {
    let locator = FilesystemLocator;
    let mut resolver = Resolver::new(ResolverInputs {
        access,
        path_dirs,
        env_profile,
        locator: &locator,
        id_gen,
    });
    resolver.resolve(request)
}

fn select_unit<'a>(units: &'a [UnitRecord], id: &str) -> Result<&'a UnitRecord, ResolutionCode> {
    let matches: Vec<_> = units.iter().filter(|u| u.id == id).collect();
    match matches.as_slice() {
        [] => Err(ResolutionCode::UnitNotFound { id: id.into() }),
        [one] => Ok(*one),
        many => {
            let mut candidates: Vec<_> = many
                .iter()
                .map(|u| {
                    u.path
                        .clone()
                        .unwrap_or_else(|| u.source.clone().unwrap_or_else(|| u.id.clone()))
                })
                .collect();
            candidates.sort();
            Err(ResolutionCode::UnitAmbiguous {
                id: id.into(),
                candidates,
            })
        }
    }
}

fn load_unit_definition(
    access: &RegistryAccess,
    unit: &UnitRecord,
    freshness: Freshness,
) -> Result<UnitDefinition, ResolutionCode> {
    // Stale/miss/provisional: authored TOML is authoritative.
    // Hit with complete routing: use projection entrypoints, still verify descriptor file.
    let use_authored = matches!(freshness, Freshness::Miss | Freshness::Stale)
        || unit.provisional
        || unit.routing_complete == Some(false);

    if !use_authored {
        let authored = find_authored_descriptors(access, &unit.id)?;
        let desc_path = match authored.as_slice() {
            [] => {
                return Err(ResolutionCode::DescriptorUnavailable {
                    message: format!(
                        "hit projection for `{}` lacks readable authored descriptor",
                        unit.id
                    ),
                });
            }
            [(path, _)] => path.clone(),
            many => {
                let mut paths: Vec<_> = many
                    .iter()
                    .map(|(p, _)| safe_workspace_path(&access.paths.workspace_root, p))
                    .collect();
                paths.sort();
                return Err(ResolutionCode::DescriptorAmbiguous {
                    id: unit.id.clone(),
                    candidates: paths,
                });
            }
        };
        if !desc_path.is_file() {
            return Err(ResolutionCode::DescriptorUnavailable {
                message: format!(
                    "descriptor missing at {}",
                    safe_workspace_path(&access.paths.workspace_root, &desc_path)
                ),
            });
        }
        return Ok(UnitDefinition {
            id: unit.id.clone(),
            path: unit.path.clone(),
            root: unit.path.clone(),
            native_manifests: unit.native_manifests.clone(),
            entrypoints: unit.entrypoints.clone(),
            descriptor_path: desc_path.display().to_string(),
            provisional: unit.provisional,
            routing_complete: unit.routing_complete.unwrap_or(true),
        });
    }

    let candidates = find_authored_descriptors(access, &unit.id)?;
    match candidates.as_slice() {
        [] => Err(ResolutionCode::DescriptorUnavailable {
            message: format!(
                "authored descriptor for `{}` not found; try `takogami scan --refresh`",
                unit.id
            ),
        }),
        [(path, authored)] => {
            if authored.id != unit.id {
                return Err(ResolutionCode::InvalidDescriptor {
                    message: format!("descriptor id `{}` != requested `{}`", authored.id, unit.id),
                });
            }
            Ok(unit_def_from_authored(path, authored))
        }
        many => {
            let mut paths: Vec<_> = many
                .iter()
                .map(|(p, _)| safe_workspace_path(&access.paths.workspace_root, p))
                .collect();
            paths.sort();
            Err(ResolutionCode::DescriptorAmbiguous {
                id: unit.id.clone(),
                candidates: paths,
            })
        }
    }
}

fn unit_def_from_authored(path: &Path, authored: &AuthoredUnitDescriptor) -> UnitDefinition {
    UnitDefinition {
        id: authored.id.clone(),
        path: Some(path.display().to_string()),
        root: authored.paths.as_ref().and_then(|p| p.root.clone()),
        native_manifests: authored
            .native
            .as_ref()
            .map(|n| n.manifests.clone())
            .unwrap_or_default(),
        entrypoints: authored.entrypoints.clone(),
        descriptor_path: path.display().to_string(),
        provisional: false,
        routing_complete: true,
    }
}

fn find_authored_descriptors(
    access: &RegistryAccess,
    unit_id: &str,
) -> Result<Vec<(PathBuf, AuthoredUnitDescriptor)>, ResolutionCode> {
    let mut roots = Vec::new();
    let colocated = access
        .paths
        .workspace_root
        .join("Build/src/workspaces/wfos/packages/ontarch/descriptors");
    if colocated.is_dir() {
        roots.push(colocated);
    }
    if let Some(parent) = access.paths.registry_root.parent() {
        let central = parent.join("descriptors");
        if central.is_dir() {
            roots.push(central);
        }
    }
    let fixture = access.paths.registry_root.join("sources/descriptors");
    if fixture.is_dir() {
        roots.push(fixture);
    }
    roots.sort();
    roots.dedup();

    let mut paths = Vec::new();
    for root in roots {
        let entries = std::fs::read_dir(&root).map_err(|_| ResolutionCode::InvalidDescriptor {
            message: format!(
                "cannot read descriptor directory `{}`",
                safe_workspace_path(&access.paths.workspace_root, &root)
            ),
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| ResolutionCode::InvalidDescriptor {
                message: format!(
                    "cannot inspect descriptor directory `{}`",
                    safe_workspace_path(&access.paths.workspace_root, &root)
                ),
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            paths.push(path);
        }
    }
    paths.sort();

    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let display = safe_workspace_path(&access.paths.workspace_root, &path);
        let text =
            std::fs::read_to_string(&path).map_err(|_| ResolutionCode::InvalidDescriptor {
                message: format!("cannot read authored descriptor `{display}`"),
            })?;
        let probed_id = probe_top_level_id(&text);
        let authored: AuthoredUnitDescriptor = match toml::from_str(&text) {
            Ok(authored) => authored,
            Err(_) if probed_id.as_deref() == Some(unit_id) || probed_id.is_none() => {
                return Err(ResolutionCode::InvalidDescriptor {
                    message: format!("invalid authored descriptor `{display}`"),
                });
            }
            Err(_) => continue,
        };
        if authored.id == unit_id {
            let key = path.canonicalize().unwrap_or(path.clone());
            if seen.insert(key) {
                found.push((path, authored));
            }
        }
    }
    Ok(found)
}

fn probe_top_level_id(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "id" {
            continue;
        }
        let value = value.trim();
        return value
            .strip_prefix('"')
            .and_then(|rest| rest.split_once('"'))
            .map(|(id, _)| id.to_string());
    }
    None
}

fn safe_workspace_path(workspace_root: &Path, path: &Path) -> String {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let normalized = candidate.canonicalize().unwrap_or(candidate);
    if normalized.starts_with(&root) {
        workspace_relative_display(&root, &normalized)
    } else {
        normalized
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("external/{name}"))
            .unwrap_or_else(|| "external/descriptor".into())
    }
}

fn classify_backend(
    entry: &NormalizedEntrypoint,
    tools: &[ToolRecord],
) -> Result<BackendKind, ResolutionCode> {
    match (entry.backend.as_deref(), entry.adapter.as_deref()) {
        (Some("native") | Some("direct"), Some("direct")) => Ok(BackendKind::NativeDirect),
        (Some("panoply"), Some("native-tool")) => Ok(BackendKind::PanoplyTool),
        (Some("moon"), Some("moon-task")) => Ok(BackendKind::MoonTask),
        (None, None) => {
            if entry.program == "moon" && entry.args.first().map(String::as_str) == Some("run") {
                Ok(BackendKind::MoonTask)
            } else if tools.iter().any(|t| {
                t.installed == Some(true)
                    && (t.id == entry.program
                        || t.detect
                            .as_deref()
                            .and_then(|d| Path::new(d).file_name())
                            .and_then(|n| n.to_str())
                            == Some(entry.program.as_str()))
            }) {
                Ok(BackendKind::PanoplyTool)
            } else {
                Ok(BackendKind::NativeDirect)
            }
        }
        (Some(_), None) | (None, Some(_)) => Err(ResolutionCode::UnsupportedBackend {
            message: "backend and adapter must both be set or both absent".into(),
        }),
        (Some(b), Some(a)) => Err(ResolutionCode::UnsupportedBackend {
            message: format!("unsupported backend/adapter pair: {b}/{a}"),
        }),
    }
}

fn authored_or_wire_labels(entry: &NormalizedEntrypoint, backend: BackendKind) -> (String, String) {
    let (wb, wa) = backend.wire_labels();
    (
        entry.backend.clone().unwrap_or_else(|| wb.into()),
        entry.adapter.clone().unwrap_or_else(|| wa.into()),
    )
}

fn validate_backend_contract(
    entry: &NormalizedEntrypoint,
    backend: BackendKind,
    tools: &[ToolRecord],
) -> Result<(), ResolutionCode> {
    match backend {
        BackendKind::MoonTask => {
            if entry.program != "moon" {
                return Err(ResolutionCode::UnsupportedBackend {
                    message: "MoonTask requires program `moon`".into(),
                });
            }
            if entry.args.first().map(String::as_str) != Some("run")
                || entry.args.get(1).map(|s| s.is_empty()).unwrap_or(true)
            {
                return Err(ResolutionCode::UnsupportedBackend {
                    message: "MoonTask argv must begin with `run` and a non-empty task".into(),
                });
            }
            Ok(())
        }
        BackendKind::PanoplyTool => {
            let ok = tools.iter().any(|t| {
                t.installed == Some(true)
                    && (t.id == entry.program
                        || t.detect
                            .as_deref()
                            .and_then(|d| Path::new(d).file_name())
                            .and_then(|n| n.to_str())
                            == Some(entry.program.as_str()))
            });
            if !ok {
                return Err(ResolutionCode::MissingExecutable {
                    message: format!(
                        "program `{}` not an installed Panoply/Ontarch tool projection",
                        entry.program
                    ),
                });
            }
            Ok(())
        }
        BackendKind::NativeDirect => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ExecutionClass;
    use crate::resolution::request::FixedIdGenerator;

    #[test]
    fn backend_pair_table() {
        let moon = NormalizedEntrypoint {
            program: "moon".into(),
            args: vec!["run".into(), "x:y".into()],
            cwd: None,
            env_keys: vec![],
            backend: Some("moon".into()),
            adapter: Some("moon-task".into()),
            source_manifests: vec![],
            required_policies: vec![],
            execution_class: ExecutionClass::Direct,
            runtime_provider: None,
            diagnostics: vec![],
        };
        assert_eq!(classify_backend(&moon, &[]).unwrap(), BackendKind::MoonTask);

        let bad = NormalizedEntrypoint {
            backend: Some("moon".into()),
            adapter: None,
            ..moon.clone()
        };
        assert!(matches!(
            classify_backend(&bad, &[]),
            Err(ResolutionCode::UnsupportedBackend { .. })
        ));
    }

    #[test]
    fn fixed_id_is_stable() {
        let mut g = FixedIdGenerator {
            id: "tkg_fixed".into(),
        };
        assert_eq!(g.next_id(), "tkg_fixed");
        assert_eq!(g.next_id(), "tkg_fixed");
    }

    #[test]
    fn resolve_twice_fixed_session_is_byte_equivalent() {
        use std::fs;
        use tempfile::tempdir;

        use crate::contracts::fingerprint_file;
        use crate::registry::{RegistryAccess, RegistryPaths};
        use crate::resolution::request::{LifecycleVerb, ResolutionRequest};

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        copy_tree(&fixture, &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["moon", "demo-bin", "rg"] {
            let p = path_dir.join(name);
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        // Build a hit units.json with matching fingerprints (same as CLI harness).
        let registry = workspace.join("registry");
        let desc_dir = registry.join("sources/descriptors");
        let mut fps = Vec::new();
        let mut units = Vec::new();
        for entry in fs::read_dir(&desc_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let rel = format!(
                "registry/sources/descriptors/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            fps.push(fingerprint_file(&workspace.join(&rel), &rel).unwrap());
            let authored: toml::Value =
                toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let id = authored["id"].as_str().unwrap().to_string();
            if id != "demo" {
                continue;
            }
            let entrypoints: serde_json::Value =
                serde_json::to_value(authored.get("entrypoints").unwrap()).unwrap();
            let native: serde_json::Value = serde_json::to_value(
                authored
                    .get("native")
                    .and_then(|n| n.get("manifests"))
                    .unwrap(),
            )
            .unwrap();
            units.push(serde_json::json!({
                "id": id,
                "kind": "package",
                "path": "demo",
                "native_manifests": native,
                "entrypoints": entrypoints,
                "source": "central",
                "provides": [],
                "requires": [],
            }));
        }
        let doc = serde_json::json!({
            "generated_at": "2026-07-21T00:00:00Z",
            "registry_generation": {
                "generated_at": "2026-07-21T00:00:00Z",
                "source_fingerprints": fps,
            },
            "summary": {"total": units.len()},
            "units": units,
        });
        fs::write(
            registry.join("units.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();

        let access = RegistryAccess::new(RegistryPaths {
            registry_root: registry,
            workspace_root: workspace.clone(),
        });
        let request = ResolutionRequest {
            session_id: "tkg_fixed_session".into(),
            unit_id: "demo".into(),
            verb: LifecycleVerb::Build,
            explicit_profile: None,
            explain: false,
            execute_requested: false,
        };
        let mut id_gen = FixedIdGenerator {
            id: "tkg_unused".into(),
        };
        let a = resolve(
            &access,
            request.clone(),
            vec![path_dir.clone()],
            None,
            &mut id_gen,
        )
        .expect("first resolve");
        let b =
            resolve(&access, request, vec![path_dir], None, &mut id_gen).expect("second resolve");

        assert_eq!(a.plan.resolved(), b.plan.resolved());
        assert_eq!(a.plan.plan_digest(), b.plan.plan_digest());
        assert_eq!(
            serde_json::to_vec(a.plan.resolved()).unwrap(),
            serde_json::to_vec(b.plan.resolved()).unwrap()
        );

        let handoff = a.policy_evaluation_input();
        assert_eq!(handoff.request().unit_id, "demo");
        assert_eq!(handoff.request().verb, "build");
        assert_eq!(handoff.request().program, "takogami");
        assert_eq!(handoff.profile().id, "workspace-dev");
        assert!(!handoff.policy_origins().is_empty());
        assert!(handoff.policy_root().exists() || !handoff.policy_root().as_os_str().is_empty());
        assert_eq!(handoff.plan().plan_digest(), a.plan.plan_digest());
        assert_eq!(
            handoff
                .policies()
                .iter()
                .map(|policy| policy.id.as_str())
                .collect::<Vec<_>>(),
            a.selected
                .policy_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }

    fn copy_tree(src: &Path, dst: &Path) {
        use std::fs;
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&to).unwrap();
                copy_tree(&entry.path(), &to);
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }
}
