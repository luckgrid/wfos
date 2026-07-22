use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "takogami",
    version,
    about = "WfOS runtime controller — discovery, routing, policy, and sessions",
    long_about = None
)]
pub struct Cli {
    /// Emit a single versioned JSON envelope on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Active profile id (required for routed commands in later stories).
    #[arg(long, global = true, value_name = "ID")]
    pub profile: Option<String>,

    /// Override operational session state root.
    #[arg(long, global = true, value_name = "PATH")]
    pub state_home: Option<PathBuf>,

    /// Disable ANSI color in human output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Enable verbose diagnostics on stderr.
    #[arg(long, short, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Discover descriptor-backed and descriptor-less units.
    Scan {
        /// Explicit refresh consent through the metadata plane.
        #[arg(long)]
        refresh: bool,
    },

    /// List units or tools from the metadata plane.
    List {
        #[command(subcommand)]
        target: ListTarget,

        /// Filter projected fields, e.g. kind=package.
        #[arg(long = "filter", value_name = "FIELD=VALUE", global = true)]
        filters: Vec<String>,
    },

    /// Show bounded metadata for one unit.
    Info { unit: String },

    /// Check controller readiness (build tools, registry, state home).
    Doctor,

    /// List native-toolchain tools (via Panoply surfaces in S3).
    Tools,

    /// Inspect or validate interface contracts.
    Interfaces {
        /// Run Ontarch contract validation.
        #[arg(long)]
        validate: bool,
    },

    /// Run a unit dev lifecycle verb.
    Dev {
        unit: String,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        execute: bool,
    },

    /// Run a unit build lifecycle verb (unit-scoped; not a separate namespace command).
    Build {
        unit: String,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        execute: bool,
    },

    /// Run a unit check lifecycle verb.
    Check {
        unit: String,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        execute: bool,
    },

    /// Project the metadata-plane dependency graph.
    Graph {
        #[arg(long, value_enum, default_value_t = GraphFormat::Text)]
        format: GraphFormat,
    },

    /// Bin/archive projection commands.
    Bin {
        #[command(subcommand)]
        sub: BinCommand,
    },

    /// Operational runtime session queries (not tracked build sessions).
    Session {
        #[command(subcommand)]
        sub: SessionCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ListTarget {
    Units,
    Tools,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GraphFormat {
    Json,
    Dot,
    Text,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BinCommand {
    /// Report bin/archive inventory (read-only).
    Report,
    /// Cleanup with an explicit mode.
    Cleanup {
        #[arg(long, value_enum, default_value_t = BinCleanupMode::ReportOnly)]
        mode: BinCleanupMode,
        #[arg(long, value_name = "PATH")]
        scope: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BinCleanupMode {
    #[value(name = "report-only")]
    ReportOnly,
    #[value(name = "dry-run")]
    DryRun,
    #[value(name = "archive")]
    Archive,
    #[value(name = "delete-approved")]
    DeleteApproved,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// List operational runtime session records.
    List,
    /// Show one operational runtime session record.
    Show { session_id: String },
    /// Show the latest operational runtime session record.
    Latest,
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Scan { .. } => "scan",
            Self::List { .. } => "list",
            Self::Info { .. } => "info",
            Self::Doctor => "doctor",
            Self::Tools => "tools",
            Self::Interfaces { .. } => "interfaces",
            Self::Dev { .. } => "dev",
            Self::Build { .. } => "build",
            Self::Check { .. } => "check",
            Self::Graph { .. } => "graph",
            Self::Bin { .. } => "bin",
            Self::Session { .. } => "session",
        }
    }

    pub fn qualified_name(&self) -> String {
        match self {
            Self::Scan { refresh } => {
                if *refresh {
                    "scan --refresh".into()
                } else {
                    "scan".into()
                }
            }
            Self::List { target, .. } => match target {
                ListTarget::Units => "list units".into(),
                ListTarget::Tools => "list tools".into(),
            },
            Self::Info { unit } => format!("info {unit}"),
            Self::Doctor => "doctor".into(),
            Self::Tools => "tools".into(),
            Self::Interfaces { validate } => {
                if *validate {
                    "interfaces validate".into()
                } else {
                    "interfaces".into()
                }
            }
            Self::Dev {
                unit,
                explain,
                execute,
            } => lifecycle_name("dev", unit, *explain, *execute),
            Self::Build {
                unit,
                explain,
                execute,
            } => lifecycle_name("build", unit, *explain, *execute),
            Self::Check {
                unit,
                explain,
                execute,
            } => lifecycle_name("check", unit, *explain, *execute),
            Self::Graph { format, .. } => format!("graph --format {}", format.as_str()),
            Self::Bin { sub } => match sub {
                BinCommand::Report => "bin report".into(),
                BinCommand::Cleanup { mode, .. } => {
                    format!("bin cleanup --mode {}", mode.as_str())
                }
            },
            Self::Session { sub } => match sub {
                SessionCommand::List => "session list".into(),
                SessionCommand::Show { session_id } => format!("session show {session_id}"),
                SessionCommand::Latest => "session latest".into(),
            },
        }
    }

    pub fn is_implemented(&self) -> bool {
        matches!(
            self,
            Self::Doctor
                | Self::Scan { .. }
                | Self::List { .. }
                | Self::Info { .. }
                | Self::Tools
                | Self::Interfaces { .. }
                | Self::Dev { .. }
                | Self::Build { .. }
                | Self::Check { .. }
        )
    }

    /// Shared lifecycle parts: (verb, unit, explain, execute).
    pub fn lifecycle_parts(&self) -> Option<(crate::resolution::LifecycleVerb, &str, bool, bool)> {
        use crate::resolution::LifecycleVerb;
        match self {
            Self::Dev {
                unit,
                explain,
                execute,
            } => Some((LifecycleVerb::Dev, unit.as_str(), *explain, *execute)),
            Self::Build {
                unit,
                explain,
                execute,
            } => Some((LifecycleVerb::Build, unit.as_str(), *explain, *execute)),
            Self::Check {
                unit,
                explain,
                execute,
            } => Some((LifecycleVerb::Check, unit.as_str(), *explain, *execute)),
            _ => None,
        }
    }
}

impl GraphFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Dot => "dot",
            Self::Text => "text",
        }
    }
}

impl BinCleanupMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReportOnly => "report-only",
            Self::DryRun => "dry-run",
            Self::Archive => "archive",
            Self::DeleteApproved => "delete-approved",
        }
    }
}

fn lifecycle_name(verb: &str, unit: &str, explain: bool, execute: bool) -> String {
    let mut parts = vec![format!("{verb} {unit}")];
    if explain {
        parts.push("--explain".into());
    }
    if execute {
        parts.push("--execute".into());
    }
    parts.join(" ")
}
