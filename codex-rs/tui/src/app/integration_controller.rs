use codex_app_server_protocol::PluginReadParams;
use codex_core::config::edit::ConfigEdit;
use codex_core::config::edit::ConfigEditsBuilder;

use super::App;
use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::AppLinkViewParams;

pub(super) struct IntegrationController;

impl IntegrationController {
    pub(super) async fn handle(app: &mut App, app_server: &mut AppServerSession, event: AppEvent) {
        match event {
            AppEvent::OpenAppLink {
                app_id,
                title,
                description,
                instructions,
                url,
                is_installed,
                is_enabled,
            } => {
                app.chat_widget.open_app_link_view(AppLinkViewParams {
                    app_id,
                    title,
                    description,
                    instructions,
                    url,
                    is_installed,
                    is_enabled,
                    suggest_reason: None,
                    suggestion_type: None,
                    elicitation_target: None,
                });
            }
            AppEvent::OpenUrlInBrowser { url } => {
                app.open_url_in_browser(url);
            }
            AppEvent::RefreshConnectors { force_refetch } => {
                app.chat_widget.refresh_connectors(force_refetch);
            }
            AppEvent::PluginInstallAuthAdvance { refresh_connectors } => {
                if refresh_connectors {
                    app.chat_widget.refresh_connectors(/*force_refetch*/ true);
                }
                app.chat_widget.advance_plugin_install_auth_flow();
            }
            AppEvent::PluginInstallAuthAbandon => {
                app.chat_widget.abandon_plugin_install_auth_flow();
            }
            AppEvent::FetchPluginsList { cwd } => {
                app.fetch_plugins_list(app_server, cwd);
            }
            AppEvent::OpenPluginDetailLoading {
                plugin_display_name,
            } => {
                app.chat_widget
                    .open_plugin_detail_loading_popup(&plugin_display_name);
            }
            AppEvent::OpenPluginInstallLoading {
                plugin_display_name,
            } => {
                app.chat_widget
                    .open_plugin_install_loading_popup(&plugin_display_name);
            }
            AppEvent::OpenPluginUninstallLoading {
                plugin_display_name,
            } => {
                app.chat_widget
                    .open_plugin_uninstall_loading_popup(&plugin_display_name);
            }
            AppEvent::PluginsLoaded { cwd, result } => {
                app.chat_widget.on_plugins_loaded(cwd, result);
            }
            AppEvent::FetchPluginDetail { cwd, params } => {
                app.fetch_plugin_detail(app_server, cwd, params);
            }
            AppEvent::PluginDetailLoaded { cwd, result } => {
                app.chat_widget.on_plugin_detail_loaded(cwd, result);
            }
            AppEvent::FetchPluginInstall {
                cwd,
                marketplace_path,
                plugin_name,
                plugin_display_name,
            } => {
                app.fetch_plugin_install(
                    app_server,
                    cwd,
                    marketplace_path,
                    plugin_name,
                    plugin_display_name,
                );
            }
            AppEvent::FetchPluginUninstall {
                cwd,
                plugin_id,
                plugin_display_name,
            } => {
                app.fetch_plugin_uninstall(app_server, cwd, plugin_id, plugin_display_name);
            }
            AppEvent::PluginInstallLoaded {
                cwd,
                marketplace_path,
                plugin_name,
                plugin_display_name,
                result,
            } => {
                let install_succeeded = result.is_ok();
                if install_succeeded {
                    if let Err(err) = app.refresh_in_memory_config_from_disk().await {
                        tracing::warn!(error = %err, "failed to refresh config after plugin install");
                    }
                    app.chat_widget.refresh_plugin_mentions();
                    app.chat_widget.submit_op(AppCommand::reload_user_config());
                }
                let should_refresh_plugin_detail = app.chat_widget.on_plugin_install_loaded(
                    cwd.clone(),
                    marketplace_path.clone(),
                    plugin_name.clone(),
                    plugin_display_name,
                    result,
                );
                if install_succeeded && app.chat_widget.config_ref().cwd.as_path() == cwd.as_path()
                {
                    app.fetch_plugins_list(app_server, cwd.clone());
                    if should_refresh_plugin_detail {
                        app.fetch_plugin_detail(
                            app_server,
                            cwd,
                            PluginReadParams {
                                marketplace_path,
                                plugin_name,
                            },
                        );
                    }
                }
            }
            AppEvent::PluginUninstallLoaded {
                cwd,
                plugin_id: _plugin_id,
                plugin_display_name,
                result,
            } => {
                let uninstall_succeeded = result.is_ok();
                if uninstall_succeeded {
                    if let Err(err) = app.refresh_in_memory_config_from_disk().await {
                        tracing::warn!(
                            error = %err,
                            "failed to refresh config after plugin uninstall"
                        );
                    }
                    app.chat_widget.refresh_plugin_mentions();
                    app.chat_widget.submit_op(AppCommand::reload_user_config());
                }
                app.chat_widget.on_plugin_uninstall_loaded(
                    cwd.clone(),
                    plugin_display_name,
                    result,
                );
                if uninstall_succeeded
                    && app.chat_widget.config_ref().cwd.as_path() == cwd.as_path()
                {
                    app.fetch_plugins_list(app_server, cwd);
                }
            }
            AppEvent::FetchMcpInventory => {
                app.fetch_mcp_inventory(app_server);
            }
            AppEvent::McpInventoryLoaded { result } => {
                app.handle_mcp_inventory_result(result);
            }
            AppEvent::StartFileSearch(query) => {
                app.file_search.on_user_query(query);
            }
            AppEvent::FileSearchResult { query, matches } => {
                app.chat_widget.apply_file_search_result(query, matches);
            }
            AppEvent::RefreshRateLimits { request_id } => {
                app.refresh_rate_limits(app_server, request_id);
            }
            AppEvent::RateLimitsLoaded { request_id, result } => match result {
                Ok(snapshots) => {
                    for snapshot in snapshots {
                        app.chat_widget.on_rate_limit_snapshot(Some(snapshot));
                    }
                    app.chat_widget.finish_status_rate_limit_refresh(request_id);
                }
                Err(err) => {
                    tracing::warn!("account/rateLimits/read failed during TUI refresh: {err}");
                    app.chat_widget.finish_status_rate_limit_refresh(request_id);
                }
            },
            AppEvent::ConnectorsLoaded { result, is_final } => {
                app.chat_widget.on_connectors_loaded(result, is_final);
            }
            AppEvent::OpenSkillsList => {
                app.chat_widget.open_skills_list();
            }
            AppEvent::OpenManageSkillsPopup => {
                app.chat_widget.open_manage_skills_popup();
            }
            AppEvent::SetSkillEnabled { path, enabled } => {
                let edits = [ConfigEdit::SetSkillConfig {
                    path: path.clone(),
                    enabled,
                }];
                match ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits(edits)
                    .apply()
                    .await
                {
                    Ok(()) => {
                        app.chat_widget.update_skill_enabled(path.clone(), enabled);
                        if let Err(err) = app.refresh_in_memory_config_from_disk().await {
                            tracing::warn!(
                                error = %err,
                                "failed to refresh config after skill toggle"
                            );
                        }
                    }
                    Err(err) => {
                        let path_display = path.display();
                        app.chat_widget.add_error_message(format!(
                            "Failed to update skill config for {path_display}: {err}"
                        ));
                    }
                }
            }
            AppEvent::SetAppEnabled { id, enabled } => {
                let edits = if enabled {
                    vec![
                        ConfigEdit::ClearPath {
                            segments: vec!["apps".to_string(), id.clone(), "enabled".to_string()],
                        },
                        ConfigEdit::ClearPath {
                            segments: vec![
                                "apps".to_string(),
                                id.clone(),
                                "disabled_reason".to_string(),
                            ],
                        },
                    ]
                } else {
                    vec![
                        ConfigEdit::SetPath {
                            segments: vec!["apps".to_string(), id.clone(), "enabled".to_string()],
                            value: false.into(),
                        },
                        ConfigEdit::SetPath {
                            segments: vec![
                                "apps".to_string(),
                                id.clone(),
                                "disabled_reason".to_string(),
                            ],
                            value: "user".into(),
                        },
                    ]
                };
                match ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits(edits)
                    .apply()
                    .await
                {
                    Ok(()) => {
                        app.chat_widget.update_connector_enabled(&id, enabled);
                        if let Err(err) = app.refresh_in_memory_config_from_disk().await {
                            tracing::warn!(error = %err, "failed to refresh config after app toggle");
                        }
                        app.chat_widget.submit_op(AppCommand::reload_user_config());
                    }
                    Err(err) => {
                        app.chat_widget.add_error_message(format!(
                            "Failed to update app config for {id}: {err}"
                        ));
                    }
                }
            }
            AppEvent::ManageSkillsClosed => {
                app.chat_widget.handle_manage_skills_closed();
            }
            _ => unreachable!("non-integration event passed to integration controller"),
        }
    }
}
