use crate::tools::handlers::ApplyPatchHandler;
use crate::tools::handlers::ContainerExecHandler;
use crate::tools::handlers::CreateGoalHandler;
use crate::tools::handlers::DynamicToolHandler;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::GetGoalHandler;
use crate::tools::handlers::GrepFilesHandler;
use crate::tools::handlers::LocalShellHandler;
use crate::tools::handlers::PlanHandler;
use crate::tools::handlers::ReadFileHandler;
use crate::tools::handlers::RequestPermissionsHandler;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::ShellCommandHandler;
use crate::tools::handlers::ShellHandler;
use crate::tools::handlers::TestSyncHandler;
use crate::tools::handlers::ToolSearchHandler;
use crate::tools::handlers::UpdateGoalHandler;
use crate::tools::handlers::ViewImageHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::agent_jobs::ReportAgentJobResultHandler;
use crate::tools::handlers::agent_jobs::SpawnAgentsOnCsvHandler;
use crate::tools::handlers::agent_jobs_spec::create_report_agent_job_result_tool;
use crate::tools::handlers::agent_jobs_spec::create_spawn_agents_on_csv_tool;
use crate::tools::handlers::apply_patch_spec::create_apply_patch_freeform_tool;
use crate::tools::handlers::apply_patch_spec::create_apply_patch_json_tool;
use crate::tools::handlers::goal_spec::create_create_goal_tool;
use crate::tools::handlers::goal_spec::create_get_goal_tool;
use crate::tools::handlers::goal_spec::create_update_goal_tool;
use crate::tools::handlers::multi_agents::CloseAgentHandler;
use crate::tools::handlers::multi_agents::ResumeAgentHandler;
use crate::tools::handlers::multi_agents::SendInputHandler;
use crate::tools::handlers::multi_agents::SpawnAgentHandler;
use crate::tools::handlers::multi_agents::WaitAgentHandler;
use crate::tools::handlers::multi_agents_spec::SpawnAgentToolOptions;
use crate::tools::handlers::multi_agents_spec::create_close_agent_tool_v1;
use crate::tools::handlers::multi_agents_spec::create_close_agent_tool_v2;
use crate::tools::handlers::multi_agents_spec::create_followup_task_tool;
use crate::tools::handlers::multi_agents_spec::create_list_agents_tool;
use crate::tools::handlers::multi_agents_spec::create_resume_agent_tool;
use crate::tools::handlers::multi_agents_spec::create_send_input_tool_v1;
use crate::tools::handlers::multi_agents_spec::create_send_message_tool;
use crate::tools::handlers::multi_agents_spec::create_spawn_agent_tool_v1;
use crate::tools::handlers::multi_agents_spec::create_spawn_agent_tool_v2;
use crate::tools::handlers::multi_agents_spec::create_wait_agent_tool_v1;
use crate::tools::handlers::multi_agents_spec::create_wait_agent_tool_v2;
use crate::tools::handlers::multi_agents_v2::CloseAgentHandler as CloseAgentHandlerV2;
use crate::tools::handlers::multi_agents_v2::FollowupTaskHandler as FollowupTaskHandlerV2;
use crate::tools::handlers::multi_agents_v2::ListAgentsHandler as ListAgentsHandlerV2;
use crate::tools::handlers::multi_agents_v2::SendMessageHandler as SendMessageHandlerV2;
use crate::tools::handlers::multi_agents_v2::SpawnAgentHandler as SpawnAgentHandlerV2;
use crate::tools::handlers::multi_agents_v2::WaitAgentHandler as WaitAgentHandlerV2;
use crate::tools::handlers::plan_spec::create_update_plan_tool;
use crate::tools::handlers::request_user_input_spec::QUESTION_TOOL_NAME;
use crate::tools::handlers::request_user_input_spec::REQUEST_USER_INPUT_TOOL_NAME;
use crate::tools::handlers::request_user_input_spec::create_question_tool;
use crate::tools::handlers::request_user_input_spec::create_request_user_input_tool;
use crate::tools::handlers::request_user_input_spec::question_tool_description;
use crate::tools::handlers::request_user_input_spec::request_user_input_tool_description;
use crate::tools::handlers::shell_spec::CommandToolOptions;
use crate::tools::handlers::shell_spec::ShellToolOptions;
use crate::tools::handlers::shell_spec::create_exec_command_tool_with_environment_id;
use crate::tools::handlers::shell_spec::create_local_shell_tool;
use crate::tools::handlers::shell_spec::create_request_permissions_tool;
use crate::tools::handlers::shell_spec::create_shell_command_tool;
use crate::tools::handlers::shell_spec::create_shell_tool;
use crate::tools::handlers::shell_spec::create_write_stdin_tool;
use crate::tools::handlers::shell_spec::request_permissions_tool_description;
use crate::tools::handlers::test_sync_spec::create_grep_files_tool;
use crate::tools::handlers::test_sync_spec::create_read_file_tool;
use crate::tools::handlers::test_sync_spec::create_test_sync_tool;
use crate::tools::handlers::tool_search_spec::create_tool_search_tool;
use crate::tools::handlers::view_image_spec::ViewImageToolOptions;
use crate::tools::handlers::view_image_spec::create_view_image_tool;
use crate::tools::hosted_spec::WebSearchToolOptions;
use crate::tools::hosted_spec::create_image_generation_tool;
use crate::tools::hosted_spec::create_web_search_tool;
use crate::tools::registry::ToolRegistryBuilder;
use crate::tools::spec_plan_types::ToolRegistryBuildParams;
use crate::tools::spec_plan_types::agent_type_description;
use codex_protocol::config_types::ModeKind;
use codex_protocol::openai_models::ApplyPatchToolType;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_tools::TOOL_SEARCH_DEFAULT_LIMIT;
use codex_tools::ToolEnvironmentMode;
use codex_tools::ToolName;
use codex_tools::ToolSearchSource;
use codex_tools::ToolSearchSourceInfo;
use codex_tools::ToolSpec;
use codex_tools::ToolsConfig;
use codex_tools::coalesce_loadable_tool_specs;
use codex_tools::collect_tool_search_source_infos;
use codex_tools::dynamic_tool_to_loadable_tool_spec;
use std::sync::Arc;

pub fn build_tool_registry_builder(
    config: &ToolsConfig,
    params: ToolRegistryBuildParams<'_>,
) -> ToolRegistryBuilder {
    let mut builder = ToolRegistryBuilder::new();
    let exec_permission_approvals_enabled = config.exec_permission_approvals_enabled;

    if config.environment_mode.has_environment() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        match &config.shell_type {
            ConfigShellToolType::Default => {
                builder.push_spec(
                    create_shell_tool(ShellToolOptions {
                        exec_permission_approvals_enabled,
                    }),
                    /*supports_parallel_tool_calls*/ true,
                    config.code_mode_enabled,
                );
            }
            ConfigShellToolType::Local => {
                builder.push_spec(
                    create_local_shell_tool(),
                    /*supports_parallel_tool_calls*/ true,
                    config.code_mode_enabled,
                );
            }
            ConfigShellToolType::UnifiedExec => {
                builder.push_spec(
                    create_exec_command_tool_with_environment_id(
                        CommandToolOptions {
                            allow_login_shell: config.allow_login_shell,
                            exec_permission_approvals_enabled,
                        },
                        include_environment_id,
                    ),
                    /*supports_parallel_tool_calls*/ true,
                    config.code_mode_enabled,
                );
                builder.push_spec(
                    create_write_stdin_tool(),
                    /*supports_parallel_tool_calls*/ false,
                    config.code_mode_enabled,
                );
                builder.register_handler(Arc::new(ExecCommandHandler));
                builder.register_handler(Arc::new(WriteStdinHandler));
            }
            ConfigShellToolType::Disabled => {}
            ConfigShellToolType::ShellCommand => {
                builder.push_spec(
                    create_shell_command_tool(CommandToolOptions {
                        allow_login_shell: config.allow_login_shell,
                        exec_permission_approvals_enabled,
                    }),
                    /*supports_parallel_tool_calls*/ true,
                    config.code_mode_enabled,
                );
            }
        }
    }

    if config.environment_mode.has_environment()
        && config.shell_type != ConfigShellToolType::Disabled
    {
        builder.register_handler(Arc::new(ShellHandler));
        builder.register_handler(Arc::new(ContainerExecHandler));
        builder.register_handler(Arc::new(LocalShellHandler));
        builder.register_handler(Arc::new(ShellCommandHandler::from(
            config.shell_command_backend,
        )));
    }

    builder.push_spec(
        create_update_plan_tool(),
        /*supports_parallel_tool_calls*/ false,
        config.code_mode_enabled,
    );
    builder.register_handler(Arc::new(PlanHandler));
    if config.goal_tools {
        builder.push_spec(
            create_get_goal_tool(),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(GetGoalHandler));
        builder.push_spec(
            create_create_goal_tool(),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(CreateGoalHandler));
        builder.push_spec(
            create_update_goal_tool(),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(UpdateGoalHandler));
    }

    builder.push_spec(
        create_question_tool(question_tool_description(
            config
                .request_user_input_available_modes
                .contains(&ModeKind::Default),
        )),
        /*supports_parallel_tool_calls*/ false,
        config.code_mode_enabled,
    );
    builder.register_handler(Arc::new(RequestUserInputHandler {
        tool_name: ToolName::plain(QUESTION_TOOL_NAME),
        available_modes: config.request_user_input_available_modes.clone(),
    }));
    builder.push_spec(
        create_request_user_input_tool(request_user_input_tool_description(
            &config.request_user_input_available_modes,
        )),
        /*supports_parallel_tool_calls*/ false,
        config.code_mode_enabled,
    );
    builder.register_handler(Arc::new(RequestUserInputHandler {
        tool_name: ToolName::plain(REQUEST_USER_INPUT_TOOL_NAME),
        available_modes: config.request_user_input_available_modes.clone(),
    }));

    if config.request_permissions_tool_enabled {
        builder.push_spec(
            create_request_permissions_tool(request_permissions_tool_description()),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(RequestPermissionsHandler));
    }

    let deferred_dynamic_tools = params
        .dynamic_tools
        .iter()
        .filter(|tool| tool.defer_loading && (config.namespace_tools || tool.namespace.is_none()))
        .collect::<Vec<_>>();
    if config.search_tool && !deferred_dynamic_tools.is_empty() {
        let mut search_source_infos = Vec::new();

        if !deferred_dynamic_tools.is_empty() {
            search_source_infos.push(ToolSearchSourceInfo {
                name: "Dynamic tools".to_string(),
                description: Some("Tools provided by the current Codex thread.".to_string()),
            });
        }

        builder.push_spec(
            create_tool_search_tool(&search_source_infos, TOOL_SEARCH_DEFAULT_LIMIT),
            /*supports_parallel_tool_calls*/ true,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(ToolSearchHandler::new(
            params.tool_search_entries.to_vec(),
        )));
    }

    if config.environment_mode.has_environment()
        && let Some(apply_patch_tool_type) = &config.apply_patch_tool_type
    {
        match apply_patch_tool_type {
            ApplyPatchToolType::Freeform => {
                builder.push_spec(
                    create_apply_patch_freeform_tool(),
                    /*supports_parallel_tool_calls*/ false,
                    config.code_mode_enabled,
                );
            }
            ApplyPatchToolType::Function => {
                builder.push_spec(
                    create_apply_patch_json_tool(),
                    /*supports_parallel_tool_calls*/ false,
                    config.code_mode_enabled,
                );
            }
        }
        builder.register_handler(Arc::new(ApplyPatchHandler));
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "test_sync_tool")
    {
        builder.push_spec(
            create_test_sync_tool(),
            /*supports_parallel_tool_calls*/ true,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(TestSyncHandler));
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "read_file")
    {
        builder.push_spec(
            create_read_file_tool(),
            /*supports_parallel_tool_calls*/ true,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(ReadFileHandler));
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "grep_files")
    {
        builder.push_spec(
            create_grep_files_tool(),
            /*supports_parallel_tool_calls*/ true,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(GrepFilesHandler));
    }

    if let Some(web_search_tool) = create_web_search_tool(WebSearchToolOptions {
        web_search_mode: config.web_search_mode,
        web_search_config: config.web_search_config.as_ref(),
        web_search_tool_type: config.web_search_tool_type,
    }) {
        builder.push_spec(
            web_search_tool,
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
    }

    if config.image_gen_tool {
        builder.push_spec(
            create_image_generation_tool("png"),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
    }

    if config.environment_mode.has_environment() {
        builder.push_spec(
            create_view_image_tool(ViewImageToolOptions {
                can_request_original_image_detail: config.can_request_original_image_detail,
            }),
            /*supports_parallel_tool_calls*/ true,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(ViewImageHandler));
    }

    if config.collab_tools {
        if config.multi_agent_v2 {
            let agent_type_description =
                agent_type_description(config, params.default_agent_type_description);
            builder.push_spec(
                create_spawn_agent_tool_v2(SpawnAgentToolOptions {
                    available_models: &config.available_models,
                    agent_type_description,
                    hide_agent_type_model_reasoning: config.hide_spawn_agent_metadata,
                    include_usage_hint: config.spawn_agent_usage_hint,
                    usage_hint_text: config.spawn_agent_usage_hint_text.clone(),
                    max_concurrent_threads_per_session: config.max_concurrent_threads_per_session,
                }),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_send_message_tool(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_followup_task_tool(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_wait_agent_tool_v2(params.wait_agent_timeouts),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_close_agent_tool_v2(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_list_agents_tool(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.register_handler(Arc::new(SpawnAgentHandlerV2));
            builder.register_handler(Arc::new(SendMessageHandlerV2));
            builder.register_handler(Arc::new(FollowupTaskHandlerV2));
            builder.register_handler(Arc::new(WaitAgentHandlerV2));
            builder.register_handler(Arc::new(CloseAgentHandlerV2));
            builder.register_handler(Arc::new(ListAgentsHandlerV2));
        } else {
            let agent_type_description =
                agent_type_description(config, params.default_agent_type_description);
            builder.push_spec(
                create_spawn_agent_tool_v1(SpawnAgentToolOptions {
                    available_models: &config.available_models,
                    agent_type_description,
                    hide_agent_type_model_reasoning: config.hide_spawn_agent_metadata,
                    include_usage_hint: config.spawn_agent_usage_hint,
                    usage_hint_text: config.spawn_agent_usage_hint_text.clone(),
                    max_concurrent_threads_per_session: config.max_concurrent_threads_per_session,
                }),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_send_input_tool_v1(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_resume_agent_tool(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.register_handler(Arc::new(ResumeAgentHandler));
            builder.push_spec(
                create_wait_agent_tool_v1(params.wait_agent_timeouts),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.push_spec(
                create_close_agent_tool_v1(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.register_handler(Arc::new(SpawnAgentHandler));
            builder.register_handler(Arc::new(SendInputHandler));
            builder.register_handler(Arc::new(WaitAgentHandler));
            builder.register_handler(Arc::new(CloseAgentHandler));
        }
    }

    if config.agent_jobs_tools {
        builder.push_spec(
            create_spawn_agents_on_csv_tool(),
            /*supports_parallel_tool_calls*/ false,
            config.code_mode_enabled,
        );
        builder.register_handler(Arc::new(SpawnAgentsOnCsvHandler));
        if config.agent_jobs_worker_tools {
            builder.push_spec(
                create_report_agent_job_result_tool(),
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
            builder.register_handler(Arc::new(ReportAgentJobResultHandler));
        }
    }

    let mut dynamic_tool_specs = Vec::new();
    for tool in params.dynamic_tools {
        match dynamic_tool_to_loadable_tool_spec(tool) {
            Ok(loadable_tool) => {
                let handler_name = ToolName::new(tool.namespace.clone(), tool.name.clone());
                dynamic_tool_specs.push(loadable_tool);
                builder.register_handler(Arc::new(DynamicToolHandler::new(handler_name)));
            }
            Err(error) => {
                tracing::error!(
                    "Failed to convert dynamic tool {:?} to OpenAI tool: {error:?}",
                    tool.name
                );
            }
        }
    }
    for spec in coalesce_loadable_tool_specs(dynamic_tool_specs) {
        let spec = spec.into();
        if config.namespace_tools || !matches!(spec, ToolSpec::Namespace(_)) {
            builder.push_spec(
                spec,
                /*supports_parallel_tool_calls*/ false,
                config.code_mode_enabled,
            );
        }
    }

    builder
}

#[cfg(test)]
#[path = "spec_plan_tests.rs"]
mod tests;
