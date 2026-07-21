use async_trait::async_trait;
use clap::Parser;
use starbase::{App, AppResult, AppSession, MainResult};
use tracing::Level;

use crate::cli::Cli;
use crate::commands;
use crate::error::ControllerError;
use crate::output::OutputSink;

#[derive(Clone)]
struct TakogamiSession {
    cli: Cli,
}

#[async_trait]
impl AppSession for TakogamiSession {
    type Error = ControllerError;

    async fn startup(&mut self) -> AppResult<Self::Error> {
        Ok(None)
    }
}

pub async fn run() -> MainResult {
    let app = App::default();
    app.setup_diagnostics();

    let cli = Cli::parse();
    let verbose = cli.verbose;
    let no_color = cli.no_color;
    let session = TakogamiSession { cli };

    let _guard = if verbose {
        Some(
            app.setup_tracing_with_defaults()
                .map_err(|err| ControllerError::internal(err.to_string()))?,
        )
    } else {
        tracing_subscriber::fmt()
            .with_max_level(Level::WARN)
            .with_writer(std::io::stderr)
            .with_ansi(!no_color)
            .try_init()
            .ok();
        None
    };

    let outcome = app
        .run(session, |session| async move { dispatch(session).await })
        .await;

    match outcome.into_miette_result() {
        Ok(code) => Ok(code),
        Err(error) => Err(miette::Report::new(error)),
    }
}

async fn dispatch(session: TakogamiSession) -> AppResult<ControllerError> {
    let sink = OutputSink {
        json: session.cli.json,
        no_color: session.cli.no_color,
    };

    let Some(command) = session.cli.command else {
        let error = ControllerError::usage("a subcommand is required; try `takogami --help`");
        let code = sink
            .emit_error("takogami", &error)
            .map_err(|err| ControllerError::internal(err.to_string()))?;
        return Ok(Some(code));
    };

    if command.is_implemented() {
        match commands::dispatch_implemented(&command, &sink, session.cli.state_home.as_deref()) {
            Ok(code) => Ok(Some(code)),
            Err(error) => {
                let code = sink
                    .emit_error(command.name(), &error)
                    .map_err(|err| ControllerError::internal(err.to_string()))?;
                Ok(Some(code))
            }
        }
    } else {
        let qualified = command.qualified_name();
        let error = ControllerError::not_implemented(&qualified);
        let code = sink
            .emit_error(command.name(), &error)
            .map_err(|err| ControllerError::internal(err.to_string()))?;
        Ok(Some(code))
    }
}
