#![deny(clippy::print_stdout, clippy::print_stderr)]
#![cfg_attr(not(feature = "runtime"), allow(dead_code, unused_imports))]

#[cfg(feature = "runtime")]
mod certs;
mod config;
#[cfg(feature = "runtime")]
mod connect_policy;
#[cfg(feature = "runtime")]
mod http_proxy;
#[cfg(feature = "runtime")]
mod mitm;
mod network_policy;
mod policy;
mod proxy;
mod reasons;
#[cfg(feature = "runtime")]
mod responses;
mod runtime;
#[cfg(feature = "runtime")]
mod socks5;
mod state;
#[cfg(feature = "runtime")]
mod upstream;

pub use config::NetworkDomainPermission;
pub use config::NetworkDomainPermissionEntry;
pub use config::NetworkDomainPermissions;
pub use config::NetworkMode;
pub use config::NetworkProxyConfig;
pub use config::NetworkUnixSocketPermission;
pub use config::NetworkUnixSocketPermissions;
pub use config::host_and_port_from_network_addr;
pub use network_policy::NetworkDecision;
pub use network_policy::NetworkDecisionSource;
pub use network_policy::NetworkPolicyDecider;
pub use network_policy::NetworkPolicyDecision;
pub use network_policy::NetworkPolicyRequest;
pub use network_policy::NetworkPolicyRequestArgs;
pub use network_policy::NetworkProtocol;
pub use policy::normalize_host;
pub use proxy::ALL_PROXY_ENV_KEYS;
pub use proxy::ALLOW_LOCAL_BINDING_ENV_KEY;
pub use proxy::Args;
#[cfg(target_os = "macos")]
pub use proxy::CODEX_PROXY_GIT_SSH_COMMAND_MARKER;
pub use proxy::DEFAULT_NO_PROXY_VALUE;
pub use proxy::NO_PROXY_ENV_KEYS;
pub use proxy::NetworkProxy;
pub use proxy::NetworkProxyBuilder;
pub use proxy::NetworkProxyHandle;
pub use proxy::PROXY_ACTIVE_ENV_KEY;
pub use proxy::PROXY_ENV_KEYS;
#[cfg(target_os = "macos")]
pub use proxy::PROXY_GIT_SSH_COMMAND_ENV_KEY;
pub use proxy::PROXY_URL_ENV_KEYS;
pub use proxy::has_proxy_url_env_vars;
pub use proxy::proxy_url_env_value;
pub use runtime::BlockedRequest;
pub use runtime::BlockedRequestArgs;
pub use runtime::BlockedRequestObserver;
pub use runtime::ConfigReloader;
pub use runtime::ConfigState;
pub use runtime::NetworkProxyState;
pub use state::NetworkProxyAuditMetadata;
pub use state::NetworkProxyConstraintError;
pub use state::NetworkProxyConstraints;
pub use state::PartialNetworkConfig;
pub use state::PartialNetworkProxyConfig;
pub use state::build_config_state;
pub use state::validate_policy_against_constraints;
