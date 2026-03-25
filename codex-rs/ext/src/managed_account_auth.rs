use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::AuthDotJson;
use codex_core::auth::load_auth_dot_json;
use codex_core::auth::save_auth;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use crate::account_pool::AccountManagementProfile;

pub const MANAGED_ACCOUNTS_RELATIVE_DIR: &str = "accounts";
const ACCOUNT_AUTH_FILE_NAME: &str = "auth.json";

#[derive(Debug, Clone, PartialEq)]
pub struct ManagedAccountSnapshot {
    pub profile: AccountManagementProfile,
    pub auth: AuthDotJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAccountAuthStore {
    codex_home: PathBuf,
}

impl ManagedAccountAuthStore {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn directory(&self) -> PathBuf {
        self.codex_home.join(MANAGED_ACCOUNTS_RELATIVE_DIR)
    }

    pub fn account_dir(&self, account_id: &str) -> PathBuf {
        self.directory().join(account_id)
    }

    pub fn account_auth_path(&self, account_id: &str) -> PathBuf {
        self.account_dir(account_id).join(ACCOUNT_AUTH_FILE_NAME)
    }

    pub fn load_account_auth(&self, account_id: &str) -> io::Result<AuthDotJson> {
        let path = self.account_auth_path(account_id);
        let contents = fs::read_to_string(path)?;
        serde_json::from_str(&contents).map_err(io::Error::other)
    }

    pub fn save_account_auth(&self, account_id: &str, auth: &AuthDotJson) -> io::Result<()> {
        let path = self.account_auth_path(account_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(auth).map_err(io::Error::other)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(contents.as_bytes())?;
        file.flush()
    }

    pub fn delete_account_auth(&self, account_id: &str) -> io::Result<()> {
        let account_dir = self.account_dir(account_id);
        match fs::remove_dir_all(account_dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

pub fn activate_managed_account(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    account_id: &str,
) -> io::Result<()> {
    let store = ManagedAccountAuthStore::new(codex_home.to_path_buf());
    let auth = store.load_account_auth(account_id)?;
    save_auth(codex_home, &auth, auth_credentials_store_mode)
}

pub fn persist_current_managed_account_snapshot(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> io::Result<Option<ManagedAccountSnapshot>> {
    let Some(snapshot) =
        load_current_managed_account_snapshot(codex_home, auth_credentials_store_mode)?
    else {
        return Ok(None);
    };
    let store = ManagedAccountAuthStore::new(codex_home.to_path_buf());
    store.save_account_auth(&snapshot.profile.id, &snapshot.auth)?;
    Ok(Some(snapshot))
}

pub fn persist_managed_account_auth_snapshot(
    codex_home: &Path,
    auth: &AuthDotJson,
    alias_override: Option<&str>,
) -> io::Result<Option<ManagedAccountSnapshot>> {
    let Some(snapshot) = managed_account_snapshot_from_auth(auth, alias_override) else {
        return Ok(None);
    };
    let store = ManagedAccountAuthStore::new(codex_home.to_path_buf());
    store.save_account_auth(&snapshot.profile.id, &snapshot.auth)?;
    Ok(Some(snapshot))
}

pub fn load_current_managed_account_snapshot(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> io::Result<Option<ManagedAccountSnapshot>> {
    let Some(auth_json) = load_auth_dot_json(codex_home, auth_credentials_store_mode)? else {
        return Ok(None);
    };
    Ok(managed_account_snapshot_from_auth(
        &auth_json, /*alias_override*/ None,
    ))
}

fn managed_account_snapshot_from_auth(
    auth: &AuthDotJson,
    alias_override: Option<&str>,
) -> Option<ManagedAccountSnapshot> {
    let tokens = auth.tokens.as_ref()?;
    let id = tokens
        .account_id
        .clone()
        .or_else(|| tokens.id_token.chatgpt_account_id.clone())?;
    let profile = AccountManagementProfile {
        id,
        alias: alias_override
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .map(ToString::to_string),
        masked_email: tokens.id_token.email.as_deref().map(mask_email),
        plan_label: tokens
            .id_token
            .get_chatgpt_plan_type()
            .map(|plan_type| plan_type.to_ascii_lowercase()),
        priority: None,
    };
    Some(ManagedAccountSnapshot {
        profile,
        auth: auth.clone(),
    })
}

fn mask_email(email: &str) -> String {
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if local.is_empty() || domain.is_empty() {
        return email.to_string();
    }

    let prefix: String = local.chars().take(3).collect();
    format!("{prefix}***@{domain}")
}

#[cfg(test)]
mod tests {
    use super::ManagedAccountAuthStore;
    use super::activate_managed_account;
    use super::persist_current_managed_account_snapshot;
    use super::persist_managed_account_auth_snapshot;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use codex_app_server_protocol::AuthMode;
    use codex_core::auth::AuthCredentialsStoreMode;
    use codex_core::auth::AuthDotJson;
    use codex_core::auth::load_auth_dot_json;
    use codex_core::auth::save_auth;
    use codex_core::token_data::TokenData;
    use serde_json::json;

    fn chatgpt_auth(account_id: &str, email: &str) -> AuthDotJson {
        AuthDotJson {
            auth_mode: Some(AuthMode::Chatgpt),
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: codex_core::token_data::parse_chatgpt_jwt_claims(&fake_jwt(
                    email, account_id, "pro",
                ))
                .expect("id token"),
                access_token: fake_jwt(email, account_id, "pro"),
                refresh_token: "refresh-token".to_string(),
                account_id: Some(account_id.to_string()),
            }),
            last_refresh: None,
        }
    }

    fn fake_jwt(email: &str, account_id: &str, plan_type: &str) -> String {
        let header = json!({"alg":"none","typ":"JWT"});
        let payload = json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_plan_type": plan_type,
            },
        });
        let encode = |value: serde_json::Value| -> String {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&value).expect("serialize"))
        };
        format!("{}.{}.sig", encode(header), encode(payload))
    }

    #[test]
    fn persist_snapshot_writes_account_auth_under_accounts_directory() {
        let tempdir = tempdir().expect("tempdir");
        let auth = chatgpt_auth("workspace-1", "user@example.com");
        save_auth(tempdir.path(), &auth, AuthCredentialsStoreMode::File).expect("save auth");

        let snapshot = persist_current_managed_account_snapshot(
            tempdir.path(),
            AuthCredentialsStoreMode::File,
        )
        .expect("persist snapshot")
        .expect("snapshot");

        assert_eq!(snapshot.profile.id, "workspace-1");
        assert_eq!(snapshot.profile.alias, None);
        assert_eq!(
            ManagedAccountAuthStore::new(tempdir.path().to_path_buf())
                .load_account_auth("workspace-1")
                .expect("load snapshot"),
            auth
        );
    }

    #[test]
    fn persist_managed_account_auth_snapshot_uses_alias_override() {
        let tempdir = tempdir().expect("tempdir");
        let auth = chatgpt_auth("workspace-1", "user@example.com");

        let snapshot =
            persist_managed_account_auth_snapshot(tempdir.path(), &auth, Some("Primary"))
                .expect("persist snapshot")
                .expect("snapshot");

        assert_eq!(snapshot.profile.alias, Some("Primary".to_string()));
        assert_eq!(
            ManagedAccountAuthStore::new(tempdir.path().to_path_buf())
                .load_account_auth("workspace-1")
                .expect("load snapshot"),
            auth
        );
    }

    #[test]
    fn activate_managed_account_restores_root_auth() {
        let tempdir = tempdir().expect("tempdir");
        let primary = chatgpt_auth("workspace-1", "primary@example.com");
        let backup = chatgpt_auth("workspace-2", "backup@example.com");
        let store = ManagedAccountAuthStore::new(tempdir.path().to_path_buf());
        store
            .save_account_auth("workspace-1", &primary)
            .expect("save primary");
        store
            .save_account_auth("workspace-2", &backup)
            .expect("save backup");

        save_auth(tempdir.path(), &primary, AuthCredentialsStoreMode::File)
            .expect("save root auth");
        activate_managed_account(
            tempdir.path(),
            AuthCredentialsStoreMode::File,
            "workspace-2",
        )
        .expect("activate backup");

        assert_eq!(
            load_auth_dot_json(tempdir.path(), AuthCredentialsStoreMode::File)
                .expect("load root auth")
                .expect("root auth"),
            backup
        );
    }

    #[test]
    fn delete_account_auth_removes_saved_snapshot() {
        let tempdir = tempdir().expect("tempdir");
        let primary = chatgpt_auth("workspace-1", "primary@example.com");
        let store = ManagedAccountAuthStore::new(tempdir.path().to_path_buf());
        store
            .save_account_auth("workspace-1", &primary)
            .expect("save primary");

        store
            .delete_account_auth("workspace-1")
            .expect("delete account auth");

        assert!(!store.account_dir("workspace-1").exists());
    }
}
