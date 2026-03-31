use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum KeyChordState {
    #[default]
    Idle,
    AwaitingCtrlXSecondKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyChordAction {
    UndoLastUserMessage,
    CopyLatestOutput,
    RespawnCurrentSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyChordResolution {
    NoMatch,
    AwaitingSecondKey,
    Matched(KeyChordAction),
    Cancelled,
    Forward(KeyEvent),
}

impl KeyChordState {
    pub(crate) fn clear(&mut self) {
        *self = Self::Idle;
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) -> KeyChordResolution {
        match self {
            Self::Idle => handle_idle_key_event(self, key_event),
            Self::AwaitingCtrlXSecondKey => handle_ctrl_x_second_key(self, key_event),
        }
    }
}

fn handle_idle_key_event(state: &mut KeyChordState, key_event: KeyEvent) -> KeyChordResolution {
    if key_event.kind != KeyEventKind::Press {
        return KeyChordResolution::NoMatch;
    }

    if key_event.code == KeyCode::Char('x') && key_event.modifiers == KeyModifiers::CONTROL {
        *state = KeyChordState::AwaitingCtrlXSecondKey;
        KeyChordResolution::AwaitingSecondKey
    } else {
        KeyChordResolution::NoMatch
    }
}

fn handle_ctrl_x_second_key(state: &mut KeyChordState, key_event: KeyEvent) -> KeyChordResolution {
    if key_event.kind != KeyEventKind::Press {
        return KeyChordResolution::AwaitingSecondKey;
    }

    let resolution = match (key_event.code, key_event.modifiers) {
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            KeyChordResolution::Matched(KeyChordAction::UndoLastUserMessage)
        }
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
            KeyChordResolution::Matched(KeyChordAction::CopyLatestOutput)
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            KeyChordResolution::Matched(KeyChordAction::RespawnCurrentSession)
        }
        (KeyCode::Char('x'), KeyModifiers::CONTROL) => KeyChordResolution::AwaitingSecondKey,
        (KeyCode::Esc, _) => KeyChordResolution::Cancelled,
        _ => KeyChordResolution::Forward(key_event),
    };

    if !matches!(resolution, KeyChordResolution::AwaitingSecondKey) {
        *state = KeyChordState::Idle;
    }

    resolution
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn ctrl_x_ctrl_u_matches_undo_last_user_message() {
        let mut state = KeyChordState::default();

        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL,)),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            KeyChordResolution::Matched(KeyChordAction::UndoLastUserMessage)
        );
        assert_eq!(state, KeyChordState::Idle);
    }

    #[test]
    fn ctrl_x_ctrl_y_matches_copy_latest_output() {
        let mut state = KeyChordState::default();

        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL,)),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            KeyChordResolution::Matched(KeyChordAction::CopyLatestOutput)
        );
        assert_eq!(state, KeyChordState::Idle);
    }

    #[test]
    fn ctrl_x_ctrl_r_matches_respawn_current_session() {
        let mut state = KeyChordState::default();

        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL,)),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            KeyChordResolution::Matched(KeyChordAction::RespawnCurrentSession)
        );
        assert_eq!(state, KeyChordState::Idle);
    }

    #[test]
    fn ctrl_x_unknown_second_key_is_forwarded_and_clears_state() {
        let mut state = KeyChordState::default();

        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL,)),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            KeyChordResolution::Forward(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        );
        assert_eq!(state, KeyChordState::Idle);
    }

    #[test]
    fn ctrl_x_release_keeps_waiting_for_second_key() {
        let mut state = KeyChordState::default();

        assert_eq!(
            state.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL,)),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(
            state.handle_key_event(KeyEvent::new_with_kind(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release,
            )),
            KeyChordResolution::AwaitingSecondKey
        );
        assert_eq!(state, KeyChordState::AwaitingCtrlXSecondKey);
    }
}
