//! Closed projection operations (bin report / cleanup modes). Graph is not a projection op.

use super::scope::ValidatedBinScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum ProjectionOperation {
    BinReport,
    BinCleanupReportOnly,
    BinCleanupDryRun,
    BinCleanupArchive,
    BinCleanupDeleteApproved,
}

impl ProjectionOperation {
    pub(crate) fn request_command_name(self) -> &'static str {
        match self {
            Self::BinReport => "bin report",
            Self::BinCleanupReportOnly => "bin cleanup --mode report-only",
            Self::BinCleanupDryRun => "bin cleanup --mode dry-run",
            Self::BinCleanupArchive => "bin cleanup --mode archive",
            Self::BinCleanupDeleteApproved => "bin cleanup --mode delete-approved",
        }
    }

    /// Canonical request argv tokens after program `takogami`.
    pub(crate) fn request_argv(self) -> Vec<String> {
        match self {
            Self::BinReport => vec!["bin".into(), "report".into()],
            Self::BinCleanupReportOnly => {
                vec![
                    "bin".into(),
                    "cleanup".into(),
                    "--mode".into(),
                    "report-only".into(),
                ]
            }
            Self::BinCleanupDryRun => {
                vec![
                    "bin".into(),
                    "cleanup".into(),
                    "--mode".into(),
                    "dry-run".into(),
                ]
            }
            Self::BinCleanupArchive => {
                vec![
                    "bin".into(),
                    "cleanup".into(),
                    "--mode".into(),
                    "archive".into(),
                ]
            }
            Self::BinCleanupDeleteApproved => {
                vec![
                    "bin".into(),
                    "cleanup".into(),
                    "--mode".into(),
                    "delete-approved".into(),
                ]
            }
        }
    }

    pub(crate) fn child_argv(self, scope: Option<&ValidatedBinScope>) -> Vec<String> {
        match self {
            Self::BinReport => vec!["bin-report".into(), "--json".into()],
            Self::BinCleanupReportOnly
            | Self::BinCleanupDryRun
            | Self::BinCleanupArchive
            | Self::BinCleanupDeleteApproved => {
                let mode = match self {
                    Self::BinCleanupReportOnly => "report-only",
                    Self::BinCleanupDryRun => "dry-run",
                    Self::BinCleanupArchive => "archive",
                    Self::BinCleanupDeleteApproved => "delete-approved",
                    Self::BinReport => unreachable!(),
                };
                let mut argv = vec!["bin-cleanup".into(), "--mode".into(), mode.into()];
                if let Some(scope) = scope {
                    argv.push("--scope".into());
                    argv.push(scope.as_str().into());
                }
                argv.push("--json".into());
                argv
            }
        }
    }

    pub(crate) fn child_supported(self) -> bool {
        matches!(self, Self::BinReport | Self::BinCleanupReportOnly)
    }

    pub(crate) fn mutation_deferred(self) -> bool {
        matches!(
            self,
            Self::BinCleanupArchive | Self::BinCleanupDeleteApproved
        )
    }

    pub(crate) fn mode_flag(self) -> Option<&'static str> {
        match self {
            Self::BinReport => None,
            Self::BinCleanupReportOnly => Some("mode=report-only"),
            Self::BinCleanupDryRun => Some("mode=dry-run"),
            Self::BinCleanupArchive => Some("mode=archive"),
            Self::BinCleanupDeleteApproved => Some("mode=delete-approved"),
        }
    }
}
