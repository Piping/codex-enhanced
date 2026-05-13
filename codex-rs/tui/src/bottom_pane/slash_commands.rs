//! Shared helpers for filtering and matching built-in slash commands.
//!
//! The same sandbox- and feature-gating rules are used by both the composer
//! and the command popup. Centralizing them here keeps those call sites small
//! and ensures they stay in sync.
use std::str::FromStr;

use codex_utils_fuzzy_match::fuzzy_match;

use crate::slash_command::SlashCommand;
use crate::slash_command::built_in_slash_commands;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BuiltinCommandFlags {
    pub(crate) collaboration_modes_enabled: bool,
    pub(crate) connectors_enabled: bool,
    pub(crate) plugins_command_enabled: bool,
    pub(crate) fast_command_enabled: bool,
    pub(crate) goal_command_enabled: bool,
    pub(crate) personality_command_enabled: bool,
    pub(crate) realtime_conversation_enabled: bool,
    pub(crate) allow_elevate_sandbox: bool,
    pub(crate) side_conversation_active: bool,
}

/// Return the built-ins that should be visible/usable for the current input.
pub(crate) fn builtins_for_input(flags: BuiltinCommandFlags) -> Vec<(&'static str, SlashCommand)> {
    built_in_slash_commands()
        .into_iter()
        .filter(|(_, cmd)| flags.allow_elevate_sandbox || *cmd != SlashCommand::ElevateSandbox)
        .filter(|(_, cmd)| {
            flags.collaboration_modes_enabled
                || !matches!(*cmd, SlashCommand::Collab | SlashCommand::Plan)
        })
        .filter(|(_, cmd)| flags.connectors_enabled || *cmd != SlashCommand::Apps)
        .filter(|(_, cmd)| flags.plugins_command_enabled || *cmd != SlashCommand::Plugins)
        .filter(|(_, cmd)| flags.fast_command_enabled || *cmd != SlashCommand::Fast)
        .filter(|(_, cmd)| flags.goal_command_enabled || *cmd != SlashCommand::Goal)
        .filter(|(_, cmd)| flags.personality_command_enabled || *cmd != SlashCommand::Personality)
        .filter(|(_, cmd)| flags.realtime_conversation_enabled || *cmd != SlashCommand::Realtime)
        .filter(|(_, cmd)| !flags.side_conversation_active || cmd.available_in_side_conversation())
        .collect()
}

/// Find a single built-in command by exact name, after applying feature gating.
///
/// Side-conversation gating is intentionally enforced by dispatch rather than exact lookup so a
/// typed command can produce a side-specific unavailable message while the popup still hides it.
pub(crate) fn find_builtin_command(name: &str, flags: BuiltinCommandFlags) -> Option<SlashCommand> {
    let cmd = SlashCommand::from_str(name).ok()?;
    builtins_for_input(BuiltinCommandFlags {
        side_conversation_active: false,
        ..flags
    })
    .into_iter()
    .any(|(_, visible_cmd)| visible_cmd == cmd)
    .then_some(cmd)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UnavailableBuiltinCommand {
    pub(crate) summary: &'static str,
    pub(crate) hint: Option<&'static str>,
}

/// Return a user-facing reason when a built-in command exists but is currently
/// unavailable because of feature or platform gating.
pub(crate) fn unavailable_builtin_command(
    name: &str,
    flags: BuiltinCommandFlags,
) -> Option<UnavailableBuiltinCommand> {
    let cmd = SlashCommand::from_str(name).ok()?;
    if find_builtin_command(name, flags).is_some() {
        return None;
    }

    match cmd {
        SlashCommand::Collab | SlashCommand::Plan if !flags.collaboration_modes_enabled => {
            Some(UnavailableBuiltinCommand {
                summary: "Collaboration modes are disabled.",
                hint: Some("Enable collaboration modes to use this command."),
            })
        }
        SlashCommand::Apps if !flags.connectors_enabled => Some(UnavailableBuiltinCommand {
            summary: "Apps are disabled.",
            hint: Some("Enable Apps support to use /apps."),
        }),
        SlashCommand::Plugins if !flags.plugins_command_enabled => {
            Some(UnavailableBuiltinCommand {
                summary: "Plugins are disabled.",
                hint: Some("Enable Plugins in /experimental to use /plugins."),
            })
        }
        SlashCommand::Fast if !flags.fast_command_enabled => Some(UnavailableBuiltinCommand {
            summary: "Fast mode is disabled.",
            hint: Some("Enable Fast mode to use /fast."),
        }),
        SlashCommand::Goal if !flags.goal_command_enabled => Some(UnavailableBuiltinCommand {
            summary: "Goals are disabled.",
            hint: Some("Enable Goals in /experimental to use /goal."),
        }),
        SlashCommand::Personality if !flags.personality_command_enabled => {
            Some(UnavailableBuiltinCommand {
                summary: "Personality selection is disabled.",
                hint: Some("Enable Personality in /experimental to use /personality."),
            })
        }
        SlashCommand::Realtime if !flags.realtime_conversation_enabled => {
            Some(UnavailableBuiltinCommand {
                summary: "Realtime conversation is disabled.",
                hint: Some("Enable Realtime conversation in /experimental to use /realtime."),
            })
        }
        SlashCommand::ElevateSandbox if !flags.allow_elevate_sandbox => {
            Some(UnavailableBuiltinCommand {
                summary: "Elevated sandbox setup is unavailable.",
                hint: Some(
                    "This command is only available when restricted Windows sandboxing is active.",
                ),
            })
        }
        _ => None,
    }
}

/// Whether any visible built-in fuzzily matches the provided prefix.
pub(crate) fn has_builtin_prefix(name: &str, flags: BuiltinCommandFlags) -> bool {
    builtins_for_input(flags)
        .into_iter()
        .any(|(command_name, _)| fuzzy_match(command_name, name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn all_enabled_flags() -> BuiltinCommandFlags {
        BuiltinCommandFlags {
            collaboration_modes_enabled: true,
            connectors_enabled: true,
            plugins_command_enabled: true,
            fast_command_enabled: true,
            goal_command_enabled: true,
            personality_command_enabled: true,
            realtime_conversation_enabled: true,
            allow_elevate_sandbox: true,
            side_conversation_active: false,
        }
    }

    #[test]
    fn debug_command_still_resolves_for_dispatch() {
        let cmd = find_builtin_command("debug-config", all_enabled_flags());
        assert_eq!(cmd, Some(SlashCommand::DebugConfig));
    }

    #[test]
    fn clear_command_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("clear", all_enabled_flags()),
            Some(SlashCommand::Clear)
        );
    }

    #[test]
    fn stop_command_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("stop", all_enabled_flags()),
            Some(SlashCommand::Stop)
        );
    }

    #[test]
    fn clean_command_alias_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("clean", all_enabled_flags()),
            Some(SlashCommand::Stop)
        );
    }

    #[test]
    fn fast_command_is_hidden_when_disabled() {
        let mut flags = all_enabled_flags();
        flags.fast_command_enabled = false;
        assert_eq!(find_builtin_command("fast", flags), None);
    }

    #[test]
    fn goal_command_is_hidden_when_disabled() {
        let mut flags = all_enabled_flags();
        flags.goal_command_enabled = false;
        assert_eq!(find_builtin_command("goal", flags), None);
    }

    #[test]
    fn goal_command_reports_unavailable_reason_when_disabled() {
        let mut flags = all_enabled_flags();
        flags.goal_command_enabled = false;
        assert_eq!(
            unavailable_builtin_command("goal", flags),
            Some(UnavailableBuiltinCommand {
                summary: "Goals are disabled.",
                hint: Some("Enable Goals in /experimental to use /goal."),
            })
        );
    }

    #[test]
    fn realtime_command_is_hidden_when_realtime_is_disabled() {
        let mut flags = all_enabled_flags();
        flags.realtime_conversation_enabled = false;
        assert_eq!(find_builtin_command("realtime", flags), None);
    }

    #[test]
    fn settings_command_stays_available_when_realtime_is_disabled() {
        let mut flags = all_enabled_flags();
        flags.realtime_conversation_enabled = false;
        assert_eq!(
            find_builtin_command("settings", flags),
            Some(SlashCommand::Settings)
        );
    }

    #[test]
    fn settings_command_stays_available_when_audio_device_selection_is_disabled() {
        assert_eq!(
            find_builtin_command("settings", all_enabled_flags()),
            Some(SlashCommand::Settings)
        );
    }

    #[test]
    fn side_conversation_hides_commands_without_side_flag() {
        let commands = builtins_for_input(BuiltinCommandFlags {
            side_conversation_active: true,
            ..all_enabled_flags()
        })
        .into_iter()
        .map(|(_, command)| command)
        .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec![
                SlashCommand::Ide,
                SlashCommand::Copy,
                SlashCommand::Raw,
                SlashCommand::Diff,
                SlashCommand::Mention,
                SlashCommand::Status,
            ]
        );
    }

    #[test]
    fn side_conversation_exact_lookup_still_resolves_hidden_commands_for_dispatch_error() {
        assert_eq!(
            find_builtin_command(
                "review",
                BuiltinCommandFlags {
                    side_conversation_active: true,
                    ..all_enabled_flags()
                },
            ),
            Some(SlashCommand::Review)
        );
    }
}
