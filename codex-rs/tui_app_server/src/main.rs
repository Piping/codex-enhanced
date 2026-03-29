use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_tui_app_server::Cli;
use codex_tui_app_server::ExitReason;
use codex_tui_app_server::run_main;
use codex_utils_cli::CliConfigOverrides;

#[derive(Parser, Debug)]
struct TopCli {
    #[clap(flatten)]
    config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    inner: Cli,
}

fn respawn_current_session(thread_id: &str, arg0_paths: &Arg0DispatchPaths) -> anyhow::Result<()> {
    let Some(exe_path) = arg0_paths.codex_self_exe.as_ref() else {
        anyhow::bail!("unable to respawn Codex: current executable path is unavailable");
    };

    let mut command = std::process::Command::new(exe_path);
    command.arg("--resume-session-id").arg(thread_id);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let error = command.exec();
        anyhow::bail!(
            "failed to respawn Codex via {}: {error}",
            exe_path.display()
        );
    }

    #[cfg(not(unix))]
    {
        command
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        command.spawn().map_err(|error| {
            anyhow::anyhow!(
                "failed to respawn Codex via {}: {error}",
                exe_path.display()
            )
        })?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let top_cli = TopCli::parse();
        let mut inner = top_cli.inner;
        inner
            .config_overrides
            .raw_overrides
            .splice(0..0, top_cli.config_overrides.raw_overrides);
        let exit_info = run_main(
            inner,
            arg0_paths.clone(),
            codex_core::config_loader::LoaderOverrides::default(),
            /*remote*/ None,
            /*remote_auth_token*/ None,
        )
        .await?;
        if matches!(exit_info.exit_reason, ExitReason::RespawnRequested) {
            let Some(thread_id) = exit_info.thread_id.as_ref() else {
                anyhow::bail!("cannot respawn Codex: current session has no thread id");
            };
            respawn_current_session(&thread_id.to_string(), &arg0_paths)?;
            return Ok(());
        }
        let token_usage = exit_info.token_usage;
        if !token_usage.is_zero() {
            println!(
                "{}",
                codex_protocol::protocol::FinalOutput::from(token_usage),
            );
        }
        Ok(())
    })
}
