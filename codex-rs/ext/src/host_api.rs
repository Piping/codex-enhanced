use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapability {
    pub name: String,
    pub version: u32,
}

impl HostCapability {
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    pub name: String,
    pub minimum_version: u32,
}

impl CapabilityRequirement {
    pub fn new(name: impl Into<String>, minimum_version: u32) -> Self {
        Self {
            name: name.into(),
            minimum_version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub capabilities: Vec<HostCapability>,
}

impl Default for HostCapabilities {
    fn default() -> Self {
        Self::codex_mvp()
    }
}

impl HostCapabilities {
    pub fn codex_mvp() -> Self {
        Self {
            capabilities: vec![
                HostCapability::new("app-start", /*version*/ 1),
                HostCapability::new("session-start", /*version*/ 1),
                HostCapability::new("before-turn-start", /*version*/ 1),
                HostCapability::new("before-tool-call", /*version*/ 1),
                HostCapability::new("after-tool-call", /*version*/ 1),
                HostCapability::new("account-routing", /*version*/ 1),
                HostCapability::new("control-panel", /*version*/ 1),
            ],
        }
    }

    pub fn supports(&self, requirement: &CapabilityRequirement) -> bool {
        self.capabilities.iter().any(|capability| {
            capability.name == requirement.name && capability.version >= requirement.minimum_version
        })
    }

    pub fn negotiate(&self, requirements: &[CapabilityRequirement]) -> PluginNegotiation {
        let mut accepted = Vec::new();
        let mut missing = Vec::new();

        for requirement in requirements {
            if self.supports(requirement) {
                accepted.push(requirement.clone());
            } else {
                missing.push(requirement.clone());
            }
        }

        PluginNegotiation { accepted, missing }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginNegotiation {
    pub accepted: Vec<CapabilityRequirement>,
    pub missing: Vec<CapabilityRequirement>,
}

impl PluginNegotiation {
    pub fn is_compatible(&self) -> bool {
        self.missing.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::CapabilityRequirement;
    use super::HostCapabilities;

    #[test]
    fn codex_mvp_capabilities_support_account_routing() {
        let capabilities = HostCapabilities::codex_mvp();
        let negotiation = capabilities.negotiate(&[
            CapabilityRequirement::new("account-routing", 1),
            CapabilityRequirement::new("control-panel", 1),
        ]);

        assert!(negotiation.is_compatible());
        assert_eq!(negotiation.accepted.len(), 2);
        assert_eq!(negotiation.missing.len(), 0);
    }

    #[test]
    fn negotiation_reports_missing_versions() {
        let capabilities = HostCapabilities::codex_mvp();
        let negotiation =
            capabilities.negotiate(&[CapabilityRequirement::new("before-turn-start", 2)]);

        assert!(!negotiation.is_compatible());
        assert_eq!(negotiation.accepted.len(), 0);
        assert_eq!(
            negotiation.missing,
            vec![CapabilityRequirement::new("before-turn-start", 2)]
        );
    }
}
