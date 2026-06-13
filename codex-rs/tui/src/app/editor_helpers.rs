use std::path::Path;

use super::App;
use crate::external_editor;
use crate::history_cell;
use crate::tui;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(super) enum ExternalEditorErrorTarget {
    History,
    ErrorMessage,
}

impl App {
    fn report_external_editor_error(&mut self, target: ExternalEditorErrorTarget, message: String) {
        match target {
            ExternalEditorErrorTarget::History => self
                .chat_widget
                .add_to_history(history_cell::new_error_event(message)),
            ExternalEditorErrorTarget::ErrorMessage => self.chat_widget.add_error_message(message),
        }
    }

    fn resolve_external_editor_commands(
        &mut self,
        target: ExternalEditorErrorTarget,
    ) -> Result<Vec<Vec<String>>, ()> {
        match external_editor::resolve_editor_commands() {
            Ok(cmds) => Ok(cmds),
            Err(external_editor::EditorError::MissingEditor) => {
                self.report_external_editor_error(
                    target,
                    "Cannot open external editor: no usable editor found in $VISUAL, $EDITOR, or `vim`."
                        .to_string(),
                );
                Err(())
            }
            Err(err) => {
                self.report_external_editor_error(target, format!("Failed to open editor: {err}"));
                Err(())
            }
        }
    }

    pub(super) async fn edit_file_with_external_editor(
        &mut self,
        tui: &mut tui::Tui,
        target: ExternalEditorErrorTarget,
        path: &Path,
    ) -> Result<(), ()> {
        let editor_cmd = self.resolve_external_editor_commands(target)?;
        let edit_result = tui
            .with_restored(tui::RestoreMode::KeepRaw, || async {
                external_editor::edit_file(path, &editor_cmd).await
            })
            .await;
        tui.frame_requester().schedule_frame();
        match edit_result {
            Ok(()) => Ok(()),
            Err(err) => {
                self.report_external_editor_error(target, format!("Failed to open editor: {err}"));
                Err(())
            }
        }
    }

    pub(super) async fn edit_seed_with_external_editor(
        &mut self,
        tui: &mut tui::Tui,
        target: ExternalEditorErrorTarget,
        seed: &str,
        suffix: &str,
    ) -> Result<String, ()> {
        let editor_cmd = self.resolve_external_editor_commands(target)?;
        let edit_result = tui
            .with_restored(tui::RestoreMode::KeepRaw, || async {
                external_editor::run_editor_with_suffix(seed, &editor_cmd, suffix).await
            })
            .await;
        tui.frame_requester().schedule_frame();
        match edit_result {
            Ok(contents) => Ok(contents),
            Err(err) => {
                self.report_external_editor_error(target, format!("Failed to open editor: {err}"));
                Err(())
            }
        }
    }
}
