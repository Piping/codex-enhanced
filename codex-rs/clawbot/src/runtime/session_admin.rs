use std::collections::HashSet;

use anyhow::Result;

use super::ClawbotRuntime;
use crate::model::ClawbotSnapshot;
use crate::model::ProviderKind;

impl ClawbotRuntime {
    pub async fn scan_provider_sessions(
        &mut self,
        provider: ProviderKind,
    ) -> Result<&ClawbotSnapshot> {
        match provider {
            ProviderKind::Feishu => {
                let Some(mut provider_runtime) = self.feishu_provider() else {
                    return self.reload();
                };

                for event in provider_runtime.scan_sessions().await? {
                    self.apply_provider_event(event)?;
                }
                self.reload()
            }
        }
    }

    pub fn clear_unbound_sessions(&mut self, provider: ProviderKind) -> Result<&ClawbotSnapshot> {
        let bindings = self.store.load_bindings()?;
        let bound_sessions = bindings
            .iter()
            .map(|binding| binding.session_ref())
            .collect::<HashSet<_>>();
        let sessions = self
            .store
            .load_sessions()?
            .into_iter()
            .filter(|session| {
                session.provider != provider || bound_sessions.contains(&session.session_ref())
            })
            .collect::<Vec<_>>();
        let unread_messages = self
            .store
            .load_unread_messages()?
            .into_iter()
            .filter(|message| {
                message.provider != provider || bound_sessions.contains(&message.session_ref())
            })
            .collect::<Vec<_>>();

        self.store.save_sessions(&sessions)?;
        self.store.save_unread_messages(&unread_messages)?;
        self.reload()
    }
}
