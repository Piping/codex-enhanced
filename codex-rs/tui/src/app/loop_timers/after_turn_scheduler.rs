use super::LoopMirroredUserTurn;
use codex_loop::LoopResponseMode;
use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterTurnRound {
    pub(crate) last_agent_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterTurnLoopUpdate {
    pub(crate) loop_id: String,
    pub(crate) rollout_path: Option<PathBuf>,
    pub(crate) last_completed_at_unix_seconds: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterTurnLoopOutput {
    pub(crate) loop_id: String,
    pub(crate) response_mode: LoopResponseMode,
    pub(crate) message: Option<String>,
    pub(crate) action: Option<String>,
    pub(crate) update: AfterTurnLoopUpdate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterTurnRoundResult {
    pub(crate) outputs: Vec<AfterTurnLoopOutput>,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AfterTurnSchedulerAction {
    RunRound(AfterTurnRound),
    SubmitFollowup(LoopMirroredUserTurn),
    Idle,
}

#[derive(Default)]
pub(crate) struct AfterTurnSchedulerState {
    pending_rounds: VecDeque<AfterTurnRound>,
    pending_followups: VecDeque<LoopMirroredUserTurn>,
    in_flight_followup: Option<LoopMirroredUserTurn>,
    round_in_flight: bool,
    current_loop_label: Option<String>,
}

impl AfterTurnSchedulerState {
    pub(crate) fn clear(&mut self) {
        self.pending_rounds.clear();
        self.pending_followups.clear();
        self.in_flight_followup = None;
        self.round_in_flight = false;
        self.current_loop_label = None;
    }

    pub(crate) fn note_turn_complete(&mut self, last_agent_message: Option<String>) {
        self.pending_rounds
            .push_back(AfterTurnRound { last_agent_message });
        self.in_flight_followup = None;
    }

    pub(crate) fn note_turn_error(&mut self) {
        self.in_flight_followup = None;
    }

    pub(crate) fn push_followups<I>(&mut self, followups: I)
    where
        I: IntoIterator<Item = LoopMirroredUserTurn>,
    {
        self.pending_followups.extend(followups);
    }

    pub(crate) fn next_action(&mut self) -> AfterTurnSchedulerAction {
        if !self.round_in_flight
            && self.in_flight_followup.is_none()
            && let Some(followup) = self.pending_followups.pop_front()
        {
            self.in_flight_followup = Some(followup.clone());
            return AfterTurnSchedulerAction::SubmitFollowup(followup);
        }
        if !self.round_in_flight
            && self.in_flight_followup.is_none()
            && let Some(round) = self.pending_rounds.pop_front()
        {
            self.round_in_flight = true;
            return AfterTurnSchedulerAction::RunRound(round);
        }
        AfterTurnSchedulerAction::Idle
    }

    pub(crate) fn note_round_completed(&mut self) {
        self.round_in_flight = false;
        self.current_loop_label = None;
    }

    pub(crate) fn note_round_progress(&mut self, loop_label: String) {
        self.current_loop_label = Some(loop_label);
    }

    pub(crate) fn status_label(&self) -> Option<String> {
        let pending_rounds = self.pending_rounds.len();
        let pending_followups =
            self.pending_followups.len() + usize::from(self.in_flight_followup.is_some());
        if !self.round_in_flight && pending_rounds == 0 && pending_followups == 0 {
            return None;
        }
        if self.round_in_flight
            && let Some(loop_label) = self.current_loop_label.as_ref()
        {
            return Some(loop_label.clone());
        }
        let status = if self.round_in_flight {
            "running"
        } else {
            "queued"
        };
        Some(format!(
            "after-turn queue ({status}) · {pending_rounds} round(s) · {pending_followups} follow-up(s)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::AfterTurnRound;
    use super::AfterTurnSchedulerAction;
    use super::AfterTurnSchedulerState;
    use crate::app::loop_timers::LoopMirroredUserTurn;
    use crate::app::loop_timers::LoopReplySource;
    use pretty_assertions::assert_eq;

    fn followup(label: &str) -> LoopMirroredUserTurn {
        LoopMirroredUserTurn {
            text: label.to_string(),
            source: LoopReplySource::new(format!("loop-{label}"), None),
        }
    }

    #[test]
    fn after_turn_round_waits_for_a_then_b_before_starting_next_round() {
        let mut scheduler = AfterTurnSchedulerState::default();
        scheduler.note_turn_complete(Some("seed".to_string()));

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::RunRound(AfterTurnRound {
                last_agent_message: Some("seed".to_string()),
            })
        );
        scheduler.note_round_completed();

        scheduler.push_followups([followup("A"), followup("B")]);

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("A"))
        );
        scheduler.note_turn_complete(Some("after A".to_string()));

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("B"))
        );
        scheduler.note_turn_complete(Some("after B".to_string()));

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::RunRound(AfterTurnRound {
                last_agent_message: Some("after A".to_string()),
            })
        );
        scheduler.note_round_completed();
        scheduler.push_followups([followup("A"), followup("B")]);

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("A"))
        );
        scheduler.note_turn_complete(Some("after A again".to_string()));

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("B"))
        );
        scheduler.note_turn_complete(Some("after B again".to_string()));

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::RunRound(AfterTurnRound {
                last_agent_message: Some("after B".to_string()),
            })
        );
    }

    #[test]
    fn followup_error_releases_scheduler_for_remaining_work() {
        let mut scheduler = AfterTurnSchedulerState::default();
        scheduler.push_followups([followup("a"), followup("b")]);

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("a"))
        );

        scheduler.note_turn_error();

        assert_eq!(
            scheduler.next_action(),
            AfterTurnSchedulerAction::SubmitFollowup(followup("b"))
        );
    }
}
