use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_tui::AppExitInfo;
use codex_tui::Cli;
use codex_tui::ExitReason;
use codex_tui::run_main;
use codex_tui::update_action::UpdateAction;
use codex_utils_cli::CliConfigOverrides;
use std::ffi::OsString;

#[derive(Parser, Debug)]
struct TopCli {
    #[clap(flatten)]
    config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    inner: Cli,
}

fn into_app_server_cli(cli: Cli) -> codex_tui_app_server::Cli {
    codex_tui_app_server::Cli {
        prompt: cli.prompt,
        images: cli.images,
        resume_picker: cli.resume_picker,
        resume_last: cli.resume_last,
        resume_session_id: cli.resume_session_id,
        resume_show_all: cli.resume_show_all,
        fork_picker: cli.fork_picker,
        fork_last: cli.fork_last,
        fork_session_id: cli.fork_session_id,
        fork_show_all: cli.fork_show_all,
        model: cli.model,
        oss: cli.oss,
        oss_provider: cli.oss_provider,
        config_profile: cli.config_profile,
        sandbox_mode: cli.sandbox_mode,
        approval_policy: cli.approval_policy,
        full_auto: cli.full_auto,
        dangerously_bypass_approvals_and_sandbox: cli.dangerously_bypass_approvals_and_sandbox,
        cwd: cli.cwd,
        web_search: cli.web_search,
        add_dir: cli.add_dir,
        no_alt_screen: cli.no_alt_screen,
        config_overrides: cli.config_overrides,
    }
}

fn into_legacy_update_action(
    action: codex_tui_app_server::update_action::UpdateAction,
) -> UpdateAction {
    match action {
        codex_tui_app_server::update_action::UpdateAction::NpmGlobalLatest => {
            UpdateAction::NpmGlobalLatest
        }
        codex_tui_app_server::update_action::UpdateAction::BunGlobalLatest => {
            UpdateAction::BunGlobalLatest
        }
        codex_tui_app_server::update_action::UpdateAction::BrewUpgrade => UpdateAction::BrewUpgrade,
    }
}

fn into_legacy_exit_reason(reason: codex_tui_app_server::ExitReason) -> ExitReason {
    match reason {
        codex_tui_app_server::ExitReason::UserRequested => ExitReason::UserRequested,
        codex_tui_app_server::ExitReason::RespawnRequested => ExitReason::RespawnRequested,
        codex_tui_app_server::ExitReason::Fatal(message) => ExitReason::Fatal(message),
    }
}

fn into_legacy_exit_info(exit_info: codex_tui_app_server::AppExitInfo) -> AppExitInfo {
    AppExitInfo {
        token_usage: exit_info.token_usage,
        thread_id: exit_info.thread_id,
        thread_name: exit_info.thread_name,
        update_action: exit_info.update_action.map(into_legacy_update_action),
        respawn_with_yolo: exit_info.respawn_with_yolo,
        exit_reason: into_legacy_exit_reason(exit_info.exit_reason),
    }
}

fn respawn_current_session(
    thread_id: &str,
    arg0_paths: &Arg0DispatchPaths,
    respawn_args: &[OsString],
    respawn_with_yolo: bool,
) -> anyhow::Result<()> {
    let Some(exe_path) = arg0_paths.codex_self_exe.as_ref() else {
        anyhow::bail!("unable to respawn Codex: current executable path is unavailable");
    };

    let mut command = std::process::Command::new(exe_path);
    command.args(build_tui_respawn_argv(
        respawn_args,
        thread_id,
        respawn_with_yolo,
    ));

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

fn approval_mode_cli_arg_name(value: codex_utils_cli::ApprovalModeCliArg) -> &'static str {
    match value {
        codex_utils_cli::ApprovalModeCliArg::Untrusted => "untrusted",
        codex_utils_cli::ApprovalModeCliArg::OnFailure => "on-failure",
        codex_utils_cli::ApprovalModeCliArg::OnRequest => "on-request",
        codex_utils_cli::ApprovalModeCliArg::Never => "never",
    }
}

fn sandbox_mode_cli_arg_name(value: codex_utils_cli::SandboxModeCliArg) -> &'static str {
    match value {
        codex_utils_cli::SandboxModeCliArg::ReadOnly => "read-only",
        codex_utils_cli::SandboxModeCliArg::WorkspaceWrite => "workspace-write",
        codex_utils_cli::SandboxModeCliArg::DangerFullAccess => "danger-full-access",
    }
}

fn push_arg_value(args: &mut Vec<OsString>, flag: &'static str, value: impl Into<OsString>) {
    args.push(flag.into());
    args.push(value.into());
}

fn build_tui_respawn_argv(
    respawn_args: &[OsString],
    thread_id: &str,
    respawn_with_yolo: bool,
) -> Vec<OsString> {
    let mut args = normalize_respawn_mode_args(respawn_args, respawn_with_yolo);
    push_arg_value(&mut args, "--resume-session-id", thread_id);
    if respawn_with_yolo {
        args.push("--yolo".into());
    }
    args
}

fn normalize_respawn_mode_args(args: &[OsString], respawn_with_yolo: bool) -> Vec<OsString> {
    let mut normalized = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let Some(arg_str) = arg.to_str() else {
            normalized.push(arg.clone());
            continue;
        };
        match arg_str {
            "--yolo" | "--dangerously-bypass-approvals-and-sandbox" => {}
            "--ask-for-approval" | "--sandbox" if respawn_with_yolo => {
                let _ = iter.next();
            }
            "--full-auto" if respawn_with_yolo => {}
            _ => normalized.push(arg.clone()),
        }
    }
    normalized
}

fn build_respawn_args(cli: &Cli) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(model) = &cli.model {
        push_arg_value(&mut args, "--model", model.clone());
    }
    if cli.oss {
        args.push("--oss".into());
    }
    if let Some(provider) = &cli.oss_provider {
        push_arg_value(&mut args, "--local-provider", provider.clone());
    }
    if let Some(profile) = &cli.config_profile {
        push_arg_value(&mut args, "--profile", profile.clone());
    }
    if let Some(sandbox_mode) = cli.sandbox_mode {
        push_arg_value(
            &mut args,
            "--sandbox",
            sandbox_mode_cli_arg_name(sandbox_mode),
        );
    }
    if let Some(approval_policy) = cli.approval_policy {
        push_arg_value(
            &mut args,
            "--ask-for-approval",
            approval_mode_cli_arg_name(approval_policy),
        );
    }
    if cli.full_auto {
        args.push("--full-auto".into());
    }
    if cli.dangerously_bypass_approvals_and_sandbox {
        args.push("--yolo".into());
    }
    if let Some(cwd) = &cli.cwd {
        push_arg_value(&mut args, "--cd", cwd.as_os_str().to_os_string());
    }
    if cli.web_search {
        args.push("--search".into());
    }
    for dir in &cli.add_dir {
        push_arg_value(&mut args, "--add-dir", dir.as_os_str().to_os_string());
    }
    if cli.no_alt_screen {
        args.push("--no-alt-screen".into());
    }
    for raw_override in &cli.config_overrides.raw_overrides {
        push_arg_value(&mut args, "-c", raw_override.clone());
    }
    args
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let top_cli = TopCli::parse();
        let mut inner = top_cli.inner;
        inner
            .config_overrides
            .raw_overrides
            .splice(0..0, top_cli.config_overrides.raw_overrides);
        let respawn_args = build_respawn_args(&inner);
        let use_app_server_tui = codex_tui::should_use_app_server_tui(&inner).await?;
        let exit_info = if use_app_server_tui {
            into_legacy_exit_info(
                codex_tui_app_server::run_main(
                    into_app_server_cli(inner),
                    arg0_paths.clone(),
                    codex_core::config_loader::LoaderOverrides::default(),
                    /*remote*/ None,
                    /*remote_auth_token*/ None,
                )
                .await?,
            )
        } else {
            run_main(
                inner,
                arg0_paths.clone(),
                codex_core::config_loader::LoaderOverrides::default(),
            )
            .await?
        };
        if matches!(exit_info.exit_reason, ExitReason::RespawnRequested) {
            let Some(thread_id) = exit_info.thread_id.as_ref() else {
                anyhow::bail!("cannot respawn Codex: current session has no thread id");
            };
            respawn_current_session(
                &thread_id.to_string(),
                &arg0_paths,
                &respawn_args,
                exit_info.respawn_with_yolo,
            )?;
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
