use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use open_lark::openlark_core::api::ApiRequest;
use open_lark::openlark_core::api::Response;
use open_lark::openlark_core::http::Transport;
use serde_json::Map;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::FeishuConfig;
use crate::config::FeishuCoordinationConfig;

const HEARTBEAT_TABLE_NAME: &str = "clawbot_coordination_heartbeat";
const FORCE_TABLE_NAME: &str = "clawbot_coordination_force";

const FIELD_TYPE_TEXT: i64 = 1;
const FIELD_TYPE_NUMBER: i64 = 2;

const HEARTBEAT_FIELD_KEY: &str = "key";
const HEARTBEAT_FIELD_APP_ID: &str = "app_id";
const HEARTBEAT_FIELD_INSTANCE_ID: &str = "instance_id";
const HEARTBEAT_FIELD_SESSION_ID: &str = "session_id";
const HEARTBEAT_FIELD_OWNER_PRIORITY: &str = "owner_priority";
const HEARTBEAT_FIELD_LAST_SEEN_MS: &str = "last_seen_ms";
const HEARTBEAT_FIELD_TTL_MS: &str = "ttl_ms";
const HEARTBEAT_FIELD_WS_STATE: &str = "ws_state";
const HEARTBEAT_FIELD_WORKSPACE_ROOT: &str = "workspace_root";

const FORCE_FIELD_KEY: &str = "key";
const FORCE_FIELD_APP_ID: &str = "app_id";
const FORCE_FIELD_TARGET_INSTANCE_ID: &str = "target_instance_id";
const FORCE_FIELD_TARGET_SESSION_ID: &str = "target_session_id";
const FORCE_FIELD_FORCE_UNTIL_MS: &str = "force_until_ms";
const FORCE_FIELD_REQUESTED_AT_MS: &str = "requested_at_ms";

const FEISHU_CODE_WRONG_BASE_TOKEN: i32 = 1_254_003;
const FEISHU_CODE_WRONG_TABLE_ID: i32 = 1_254_004;
const FEISHU_CODE_WRONG_FIELD_ID: i32 = 1_254_009;
const FEISHU_CODE_BASE_TOKEN_NOT_FOUND: i32 = 1_254_040;
const FEISHU_CODE_TABLE_ID_NOT_FOUND: i32 = 1_254_041;
const FEISHU_CODE_FIELD_ID_NOT_FOUND: i32 = 1_254_044;
const FEISHU_CODE_PERMISSION_DENIED: i32 = 1_254_302;

const HEARTBEAT_FIELDS: [RequiredField; 9] = [
    RequiredField::new(HEARTBEAT_FIELD_KEY, FIELD_TYPE_TEXT),
    RequiredField::new(HEARTBEAT_FIELD_APP_ID, FIELD_TYPE_TEXT),
    RequiredField::new(HEARTBEAT_FIELD_INSTANCE_ID, FIELD_TYPE_TEXT),
    RequiredField::new(HEARTBEAT_FIELD_SESSION_ID, FIELD_TYPE_TEXT),
    RequiredField::new(HEARTBEAT_FIELD_OWNER_PRIORITY, FIELD_TYPE_NUMBER),
    RequiredField::new(HEARTBEAT_FIELD_LAST_SEEN_MS, FIELD_TYPE_NUMBER),
    RequiredField::new(HEARTBEAT_FIELD_TTL_MS, FIELD_TYPE_NUMBER),
    RequiredField::new(HEARTBEAT_FIELD_WS_STATE, FIELD_TYPE_TEXT),
    RequiredField::new(HEARTBEAT_FIELD_WORKSPACE_ROOT, FIELD_TYPE_TEXT),
];

const FORCE_FIELDS: [RequiredField; 6] = [
    RequiredField::new(FORCE_FIELD_KEY, FIELD_TYPE_TEXT),
    RequiredField::new(FORCE_FIELD_APP_ID, FIELD_TYPE_TEXT),
    RequiredField::new(FORCE_FIELD_TARGET_INSTANCE_ID, FIELD_TYPE_TEXT),
    RequiredField::new(FORCE_FIELD_TARGET_SESSION_ID, FIELD_TYPE_TEXT),
    RequiredField::new(FORCE_FIELD_FORCE_UNTIL_MS, FIELD_TYPE_NUMBER),
    RequiredField::new(FORCE_FIELD_REQUESTED_AT_MS, FIELD_TYPE_NUMBER),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WebsocketOwnershipState {
    Idle,
    Connected,
    BackingOff,
}

impl WebsocketOwnershipState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connected => "connected",
            Self::BackingOff => "backing_off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LeadershipSnapshot {
    pub is_leader: bool,
    pub leader_instance_id: String,
    pub leader_session_id: String,
    pub forced_instance_id: Option<String>,
}

impl LeadershipSnapshot {
    pub fn standby_message(&self) -> String {
        let forced = self
            .forced_instance_id
            .as_deref()
            .map(|instance_id| format!("; force owner {instance_id}"))
            .unwrap_or_default();
        format!(
            "leader is {} (session {}){forced}",
            self.leader_instance_id, self.leader_session_id
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct FeishuBaseCoordinator {
    client: FeishuBaseClient,
    app_id: String,
    workspace_root: String,
    instance_id: String,
    session_id: String,
    owner_priority: i64,
    heartbeat_interval: Duration,
    heartbeat_ttl: Duration,
    force_connect: bool,
}

impl FeishuBaseCoordinator {
    pub(super) fn new(workspace_root: &Path, config: &FeishuConfig) -> Result<Option<Self>> {
        let Some(coordination) = config
            .coordination
            .clone()
            .filter(FeishuCoordinationConfig::is_configured)
        else {
            return Ok(None);
        };

        let core_config = super::runtime_loop::build_websocket_config(config)?
            .build_core_config_with_token_provider();
        let now_ms = unix_timestamp_ms_now()?;
        let workspace_root_display = workspace_root.display().to_string();
        let instance_id = coordination
            .instance_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_instance_id(&workspace_root_display));
        let session_id = format!("p{}-{now_ms}", std::process::id());
        let owner_priority = coordination.owner_priority;
        let heartbeat_interval = coordination.heartbeat_interval();
        let heartbeat_ttl = coordination.heartbeat_ttl();
        let force_connect = coordination.force_connect;

        Ok(Some(Self {
            client: FeishuBaseClient::new(core_config, coordination),
            app_id: config.app_id.clone(),
            workspace_root: workspace_root_display,
            instance_id,
            session_id,
            owner_priority,
            heartbeat_interval,
            heartbeat_ttl,
            force_connect,
        }))
    }

    pub(super) fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub(super) async fn refresh_leadership(
        &self,
        websocket_state: WebsocketOwnershipState,
    ) -> Result<LeadershipSnapshot> {
        let now_ms = unix_timestamp_ms_now()?;
        let heartbeat = HeartbeatLease {
            key: heartbeat_key(&self.app_id, &self.instance_id),
            app_id: self.app_id.clone(),
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            owner_priority: self.owner_priority,
            last_seen_ms: now_ms,
            ttl_ms: duration_to_millis_i64(self.heartbeat_ttl)?,
            ws_state: websocket_state.as_str().to_string(),
            workspace_root: self.workspace_root.clone(),
        };
        self.client.upsert_heartbeat(&heartbeat).await?;
        self.client
            .sync_force_intent(
                &self.app_id,
                &self.instance_id,
                &self.session_id,
                now_ms,
                heartbeat.ttl_ms,
                self.force_connect,
            )
            .await?;

        let leases = self.client.list_heartbeats(&self.app_id).await?;
        let force_intent = self.client.get_force_intent(&self.app_id).await?;
        select_leader(&self.instance_id, now_ms, &leases, force_intent.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedCoordinationTables {
    heartbeat_table_id: String,
    force_table_id: String,
}

#[derive(Debug, Clone)]
struct FeishuBaseClient {
    config: open_lark::openlark_core::config::Config,
    base_token: String,
    configured_heartbeat_table_id: Option<String>,
    configured_force_table_id: Option<String>,
    schema: Arc<Mutex<Option<ResolvedCoordinationTables>>>,
}

impl FeishuBaseClient {
    fn new(
        config: open_lark::openlark_core::config::Config,
        coordination: FeishuCoordinationConfig,
    ) -> Self {
        Self {
            config,
            base_token: coordination.base_token,
            configured_heartbeat_table_id: trimmed_non_empty(&coordination.heartbeat_table_id),
            configured_force_table_id: trimmed_non_empty(&coordination.force_table_id),
            schema: Arc::new(Mutex::new(None)),
        }
    }

    async fn upsert_heartbeat(&self, heartbeat: &HeartbeatLease) -> Result<()> {
        let tables = self.ensure_schema().await?;
        let fields = heartbeat.to_fields();
        match self
            .find_heartbeat_record_by_key(&tables.heartbeat_table_id, &heartbeat.key)
            .await?
        {
            Some(record) => {
                self.update_record(&tables.heartbeat_table_id, &record.record_id, fields)
                    .await
                    .context("failed to update Feishu Base heartbeat record")?;
            }
            None => {
                self.create_record(&tables.heartbeat_table_id, fields)
                    .await
                    .context("failed to create Feishu Base heartbeat record")?;
            }
        }
        Ok(())
    }

    async fn list_heartbeats(&self, app_id: &str) -> Result<Vec<HeartbeatLease>> {
        let tables = self.ensure_schema().await?;
        let mut heartbeats = Vec::new();
        for record in self.list_records(&tables.heartbeat_table_id).await? {
            if let Some(heartbeat) = HeartbeatLease::from_record(record)?
                && heartbeat.app_id == app_id
            {
                heartbeats.push(heartbeat);
            }
        }
        Ok(heartbeats)
    }

    async fn sync_force_intent(
        &self,
        app_id: &str,
        target_instance_id: &str,
        target_session_id: &str,
        now_ms: i64,
        ttl_ms: i64,
        force_connect: bool,
    ) -> Result<()> {
        let tables = self.ensure_schema().await?;
        let key = force_key(app_id);
        let existing = self
            .find_force_record_by_key(&tables.force_table_id, &key)
            .await?;
        let force_until_ms = if force_connect {
            now_ms + ttl_ms
        } else {
            now_ms
        };
        let should_write = force_connect
            || existing.as_ref().is_some_and(|record| {
                record.target_instance_id == target_instance_id && record.force_until_ms > now_ms
            });
        if !should_write {
            return Ok(());
        }

        let fields = ForceIntentRecord {
            record_id: existing
                .as_ref()
                .and_then(|record| record.record_id.clone()),
            key,
            app_id: app_id.to_string(),
            target_instance_id: target_instance_id.to_string(),
            target_session_id: target_session_id.to_string(),
            force_until_ms,
            requested_at_ms: now_ms,
        }
        .to_fields();
        if let Some(existing) = existing {
            self.update_record(
                &tables.force_table_id,
                &existing
                    .record_id
                    .clone()
                    .context("missing Feishu Base force intent record id")?,
                fields,
            )
            .await
            .context("failed to update Feishu Base force intent record")?;
        } else {
            self.create_record(&tables.force_table_id, fields)
                .await
                .context("failed to create Feishu Base force intent record")?;
        }
        Ok(())
    }

    async fn get_force_intent(&self, app_id: &str) -> Result<Option<ForceIntentRecord>> {
        let tables = self.ensure_schema().await?;
        self.find_force_record_by_key(&tables.force_table_id, &force_key(app_id))
            .await
    }

    async fn ensure_schema(&self) -> Result<ResolvedCoordinationTables> {
        let mut cached = self.schema.lock().await;
        if let Some(schema) = cached.clone() {
            return Ok(schema);
        }

        let mut tables = self.list_tables().await?;
        let heartbeat_table = self
            .resolve_or_create_table(
                &mut tables,
                self.configured_heartbeat_table_id.as_deref(),
                HEARTBEAT_TABLE_NAME,
            )
            .await?;
        self.ensure_required_fields(&heartbeat_table, &HEARTBEAT_FIELDS)
            .await?;

        let force_table = self
            .resolve_or_create_table(
                &mut tables,
                self.configured_force_table_id.as_deref(),
                FORCE_TABLE_NAME,
            )
            .await?;
        self.ensure_required_fields(&force_table, &FORCE_FIELDS)
            .await?;

        let schema = ResolvedCoordinationTables {
            heartbeat_table_id: heartbeat_table.table_id,
            force_table_id: force_table.table_id,
        };
        *cached = Some(schema.clone());
        Ok(schema)
    }

    async fn resolve_or_create_table(
        &self,
        tables: &mut Vec<BaseTable>,
        configured_table_id: Option<&str>,
        table_name: &str,
    ) -> Result<BaseTable> {
        if let Some(table_id) = configured_table_id
            && let Some(table) = tables.iter().find(|table| table.table_id == table_id)
        {
            return Ok(table.clone());
        }

        if let Some(table) = choose_named_table(tables, table_name) {
            return Ok(table);
        }

        self.create_table(table_name).await?;
        *tables = self.list_tables().await?;
        choose_named_table(tables, table_name).ok_or_else(|| {
            anyhow!(
                "Feishu Base coordination created table {table_name} but could not resolve its table_id"
            )
        })
    }

    async fn ensure_required_fields(
        &self,
        table: &BaseTable,
        required_fields: &[RequiredField],
    ) -> Result<()> {
        let fields = self.list_fields(&table.table_id).await?;
        for required in required_fields {
            match fields
                .iter()
                .find(|field| field.field_name == required.field_name)
            {
                Some(existing) if existing.field_type == required.field_type => {}
                Some(existing) => {
                    return Err(anyhow!(
                        "Feishu Base coordination table {} ({}) has field {} with type {} but expected {}. Repair the table schema or clear the configured table id so clawbot can recreate it.",
                        table.name,
                        table.table_id,
                        required.field_name,
                        field_type_name(existing.field_type),
                        field_type_name(required.field_type),
                    ));
                }
                None => {
                    self.create_field(&table.table_id, required)
                        .await
                        .with_context(|| {
                            format!(
                                "failed to create Feishu Base coordination field {} on table {} ({})",
                                required.field_name, table.name, table.table_id
                            )
                        })?;
                }
            }
        }
        Ok(())
    }

    async fn find_force_record_by_key(
        &self,
        table_id: &str,
        key: &str,
    ) -> Result<Option<ForceIntentRecord>> {
        let mut matches = Vec::new();
        for record in self.list_records(table_id).await? {
            if let Some(force_intent) = ForceIntentRecord::from_record(record)?
                && force_intent.key == key
            {
                matches.push(force_intent);
            }
        }
        matches.sort_by(|left, right| {
            left.requested_at_ms
                .cmp(&right.requested_at_ms)
                .then(left.record_id.cmp(&right.record_id))
        });
        Ok(matches.pop())
    }

    async fn find_heartbeat_record_by_key(
        &self,
        table_id: &str,
        key: &str,
    ) -> Result<Option<BaseRecord>> {
        let mut matches = self
            .list_records(table_id)
            .await?
            .into_iter()
            .filter(|record| {
                record
                    .fields
                    .as_object()
                    .and_then(|fields| string_field(fields, HEARTBEAT_FIELD_KEY))
                    .is_some_and(|record_key| record_key == key)
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.record_id.cmp(&right.record_id));
        Ok(matches.pop())
    }

    async fn list_tables(&self) -> Result<Vec<BaseTable>> {
        let mut page_token = None;
        let mut tables = Vec::new();
        loop {
            let mut request: ApiRequest<Value> =
                ApiRequest::get(tables_url(&self.base_token)).query("page_size", "100");
            if let Some(token) = page_token.clone() {
                request = request.query("page_token", token);
            }
            let response = self
                .request_json("list Feishu Base tables", request)
                .await?;
            let payload = paged_payload(
                response
                    .data
                    .as_ref()
                    .context("Feishu Base table list response is missing data")?,
                "table list",
            )?;
            if let Some(items) = payload.get("items").and_then(Value::as_array) {
                for item in items {
                    if let Some(table) = BaseTable::from_value(item) {
                        tables.push(table);
                    }
                }
            }
            let has_more = payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_more {
                break;
            }
            let Some(next_page_token) = payload
                .get("page_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
            else {
                break;
            };
            page_token = Some(next_page_token);
        }
        Ok(tables)
    }

    async fn create_table(&self, table_name: &str) -> Result<()> {
        let request: ApiRequest<Value> =
            ApiRequest::post(tables_url(&self.base_token)).json_body(&serde_json::json!({
                "table": {
                    "name": table_name,
                    "fields": [{
                        "field_name": HEARTBEAT_FIELD_KEY,
                        "type": FIELD_TYPE_TEXT,
                    }],
                }
            }));
        self.request_json(&format!("create Feishu Base table {table_name}"), request)
            .await?;
        Ok(())
    }

    async fn list_fields(&self, table_id: &str) -> Result<Vec<BaseField>> {
        let mut page_token = None;
        let mut fields = Vec::new();
        loop {
            let mut request: ApiRequest<Value> =
                ApiRequest::get(fields_url(&self.base_token, table_id)).query("page_size", "100");
            if let Some(token) = page_token.clone() {
                request = request.query("page_token", token);
            }
            let response = self
                .request_json(
                    &format!("list Feishu Base fields for table {table_id}"),
                    request,
                )
                .await?;
            let payload = paged_payload(
                response
                    .data
                    .as_ref()
                    .context("Feishu Base field list response is missing data")?,
                "field list",
            )?;
            if let Some(items) = payload.get("items").and_then(Value::as_array) {
                for item in items {
                    if let Some(field) = BaseField::from_value(item) {
                        fields.push(field);
                    }
                }
            }
            let has_more = payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_more {
                break;
            }
            let Some(next_page_token) = payload
                .get("page_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
            else {
                break;
            };
            page_token = Some(next_page_token);
        }
        Ok(fields)
    }

    async fn create_field(&self, table_id: &str, field: &RequiredField) -> Result<()> {
        let request: ApiRequest<Value> = ApiRequest::post(fields_url(&self.base_token, table_id))
            .json_body(&serde_json::json!({
                "field_name": field.field_name,
                "type": field.field_type,
            }));
        self.request_json(
            &format!(
                "create Feishu Base field {} on table {table_id}",
                field.field_name
            ),
            request,
        )
        .await?;
        Ok(())
    }

    async fn list_records(&self, table_id: &str) -> Result<Vec<BaseRecord>> {
        let mut page_token = None;
        let mut records = Vec::new();
        loop {
            let mut request: ApiRequest<Value> =
                ApiRequest::get(records_url(&self.base_token, table_id)).query("page_size", "500");
            if let Some(token) = page_token.clone() {
                request = request.query("page_token", token);
            }
            let response = self
                .request_json(
                    &format!("list Feishu Base records for table {table_id}"),
                    request,
                )
                .await?;
            let payload = paged_payload(
                response
                    .data
                    .as_ref()
                    .context("Feishu Base record list response is missing data")?,
                "record list",
            )?;
            if let Some(items) = payload.get("items").and_then(Value::as_array) {
                for item in items {
                    if let Some(record) = BaseRecord::from_value(item) {
                        records.push(record);
                    }
                }
            }
            let has_more = payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_more {
                break;
            }
            let Some(next_page_token) = payload
                .get("page_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
            else {
                break;
            };
            page_token = Some(next_page_token);
        }
        Ok(records)
    }

    async fn create_record(&self, table_id: &str, fields: Value) -> Result<()> {
        let request: ApiRequest<Value> = ApiRequest::post(records_url(&self.base_token, table_id))
            .json_body(&serde_json::json!({ "fields": fields }));
        self.request_json(
            &format!("create Feishu Base record in table {table_id}"),
            request,
        )
        .await?;
        Ok(())
    }

    async fn update_record(&self, table_id: &str, record_id: &str, fields: Value) -> Result<()> {
        let request: ApiRequest<Value> =
            ApiRequest::put(record_url(&self.base_token, table_id, record_id))
                .json_body(&serde_json::json!({ "fields": fields }));
        self.request_json(
            &format!("update Feishu Base record {record_id} in table {table_id}"),
            request,
        )
        .await?;
        Ok(())
    }

    async fn request_json(
        &self,
        operation: &str,
        request: ApiRequest<Value>,
    ) -> Result<Response<Value>> {
        let response = Transport::<Value>::request(request, &self.config, Some(Default::default()))
            .await
            .with_context(|| format!("failed to {operation}"))?;
        if response.is_success() {
            Ok(response)
        } else {
            Err(classify_feishu_api_error(
                operation,
                &self.base_token,
                &response,
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequiredField {
    field_name: &'static str,
    field_type: i64,
}

impl RequiredField {
    const fn new(field_name: &'static str, field_type: i64) -> Self {
        Self {
            field_name,
            field_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BaseTable {
    table_id: String,
    name: String,
}

impl BaseTable {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            table_id: value.get("table_id")?.as_str()?.to_string(),
            name: value.get("name")?.as_str()?.trim().to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BaseField {
    field_name: String,
    field_type: i64,
}

impl BaseField {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            field_name: value.get("field_name")?.as_str()?.trim().to_string(),
            field_type: value.get("type")?.as_i64()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BaseRecord {
    record_id: String,
    fields: Value,
}

impl BaseRecord {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            record_id: value.get("record_id")?.as_str()?.to_string(),
            fields: value.get("fields")?.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeartbeatLease {
    key: String,
    app_id: String,
    instance_id: String,
    session_id: String,
    owner_priority: i64,
    last_seen_ms: i64,
    ttl_ms: i64,
    ws_state: String,
    workspace_root: String,
}

impl HeartbeatLease {
    fn from_record(record: BaseRecord) -> Result<Option<Self>> {
        let Some(fields) = record.fields.as_object() else {
            return Ok(None);
        };
        let Some(app_id) = string_field(fields, HEARTBEAT_FIELD_APP_ID) else {
            return Ok(None);
        };
        let Some(instance_id) = string_field(fields, HEARTBEAT_FIELD_INSTANCE_ID) else {
            return Ok(None);
        };
        let Some(session_id) = string_field(fields, HEARTBEAT_FIELD_SESSION_ID) else {
            return Ok(None);
        };
        Ok(Some(Self {
            key: string_field(fields, HEARTBEAT_FIELD_KEY)
                .unwrap_or_else(|| heartbeat_key(&app_id, &instance_id)),
            app_id,
            instance_id,
            session_id,
            owner_priority: integer_field(fields, HEARTBEAT_FIELD_OWNER_PRIORITY).unwrap_or(0),
            last_seen_ms: integer_field(fields, HEARTBEAT_FIELD_LAST_SEEN_MS).unwrap_or(0),
            ttl_ms: integer_field(fields, HEARTBEAT_FIELD_TTL_MS).unwrap_or(0),
            ws_state: string_field(fields, HEARTBEAT_FIELD_WS_STATE).unwrap_or_default(),
            workspace_root: string_field(fields, HEARTBEAT_FIELD_WORKSPACE_ROOT)
                .unwrap_or_default(),
        }))
    }

    fn is_active(&self, now_ms: i64) -> bool {
        self.last_seen_ms + self.ttl_ms >= now_ms
    }

    fn to_fields(&self) -> Value {
        serde_json::json!({
            HEARTBEAT_FIELD_KEY: self.key,
            HEARTBEAT_FIELD_APP_ID: self.app_id,
            HEARTBEAT_FIELD_INSTANCE_ID: self.instance_id,
            HEARTBEAT_FIELD_SESSION_ID: self.session_id,
            HEARTBEAT_FIELD_OWNER_PRIORITY: self.owner_priority,
            HEARTBEAT_FIELD_LAST_SEEN_MS: self.last_seen_ms,
            HEARTBEAT_FIELD_TTL_MS: self.ttl_ms,
            HEARTBEAT_FIELD_WS_STATE: self.ws_state,
            HEARTBEAT_FIELD_WORKSPACE_ROOT: self.workspace_root,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForceIntentRecord {
    record_id: Option<String>,
    key: String,
    app_id: String,
    target_instance_id: String,
    target_session_id: String,
    force_until_ms: i64,
    requested_at_ms: i64,
}

impl ForceIntentRecord {
    fn from_record(record: BaseRecord) -> Result<Option<Self>> {
        let Some(fields) = record.fields.as_object() else {
            return Ok(None);
        };
        let Some(app_id) = string_field(fields, FORCE_FIELD_APP_ID) else {
            return Ok(None);
        };
        let Some(target_instance_id) = string_field(fields, FORCE_FIELD_TARGET_INSTANCE_ID) else {
            return Ok(None);
        };
        let Some(target_session_id) = string_field(fields, FORCE_FIELD_TARGET_SESSION_ID) else {
            return Ok(None);
        };
        Ok(Some(Self {
            record_id: Some(record.record_id),
            key: string_field(fields, FORCE_FIELD_KEY).unwrap_or_else(|| force_key(&app_id)),
            app_id,
            target_instance_id,
            target_session_id,
            force_until_ms: integer_field(fields, FORCE_FIELD_FORCE_UNTIL_MS).unwrap_or(0),
            requested_at_ms: integer_field(fields, FORCE_FIELD_REQUESTED_AT_MS).unwrap_or(0),
        }))
    }

    fn is_active(&self, now_ms: i64) -> bool {
        self.force_until_ms >= now_ms
    }

    fn to_fields(&self) -> Value {
        serde_json::json!({
            FORCE_FIELD_KEY: self.key,
            FORCE_FIELD_APP_ID: self.app_id,
            FORCE_FIELD_TARGET_INSTANCE_ID: self.target_instance_id,
            FORCE_FIELD_TARGET_SESSION_ID: self.target_session_id,
            FORCE_FIELD_FORCE_UNTIL_MS: self.force_until_ms,
            FORCE_FIELD_REQUESTED_AT_MS: self.requested_at_ms,
        })
    }
}

fn select_leader(
    current_instance_id: &str,
    now_ms: i64,
    leases: &[HeartbeatLease],
    force_intent: Option<&ForceIntentRecord>,
) -> Result<LeadershipSnapshot> {
    let mut active = leases
        .iter()
        .filter(|lease| lease.is_active(now_ms))
        .cloned()
        .collect::<Vec<_>>();
    active.sort_by(|left, right| {
        right
            .owner_priority
            .cmp(&left.owner_priority)
            .then(left.instance_id.cmp(&right.instance_id))
            .then(left.session_id.cmp(&right.session_id))
    });
    let leader =
        if let Some(force_intent) = force_intent.filter(|intent| intent.is_active(now_ms)) {
            if let Some(forced_lease) = active
                .iter()
                .find(|lease| lease.instance_id == force_intent.target_instance_id)
            {
                forced_lease.clone()
            } else {
                active.first().cloned().ok_or_else(|| {
                    anyhow!("no active Feishu Base coordination heartbeat rows found")
                })?
            }
        } else {
            active
                .first()
                .cloned()
                .ok_or_else(|| anyhow!("no active Feishu Base coordination heartbeat rows found"))?
        };
    Ok(LeadershipSnapshot {
        is_leader: leader.instance_id == current_instance_id,
        leader_instance_id: leader.instance_id,
        leader_session_id: leader.session_id,
        forced_instance_id: force_intent
            .filter(|intent| intent.is_active(now_ms))
            .map(|intent| intent.target_instance_id.clone()),
    })
}

fn choose_named_table(tables: &[BaseTable], table_name: &str) -> Option<BaseTable> {
    tables
        .iter()
        .filter(|table| table.name == table_name)
        .cloned()
        .min_by(|left, right| left.table_id.cmp(&right.table_id))
}

fn classify_feishu_api_error(
    operation: &str,
    base_token: &str,
    response: &Response<Value>,
) -> anyhow::Error {
    let code = response.code();
    let msg = response.msg().trim();
    let request_id = response
        .raw()
        .request_id
        .as_deref()
        .map(|id| format!(", request_id {id}"))
        .unwrap_or_default();

    match code {
        FEISHU_CODE_PERMISSION_DENIED => anyhow!(
            "Feishu Base coordination could not {operation}: permission denied (code {code}{request_id}): {msg}. Grant the app edit/admin access on base {base_token} or add it to a Base role with write permission."
        ),
        FEISHU_CODE_WRONG_BASE_TOKEN | FEISHU_CODE_BASE_TOKEN_NOT_FOUND => anyhow!(
            "Feishu Base coordination could not {operation}: base_token {base_token} is invalid or inaccessible (code {code}{request_id}): {msg}. Check [feishu.coordination].base_token and make sure the app can access that Base."
        ),
        FEISHU_CODE_WRONG_TABLE_ID | FEISHU_CODE_TABLE_ID_NOT_FOUND => anyhow!(
            "Feishu Base coordination could not {operation}: the configured table id is invalid (code {code}{request_id}): {msg}. Clear or repair heartbeat_table_id / force_table_id so clawbot can resolve or recreate its coordination tables."
        ),
        FEISHU_CODE_WRONG_FIELD_ID | FEISHU_CODE_FIELD_ID_NOT_FOUND => anyhow!(
            "Feishu Base coordination could not {operation}: the coordination table schema is missing a required field (code {code}{request_id}): {msg}. Repair the Base schema or let clawbot recreate the coordination tables."
        ),
        _ => anyhow!(
            "Feishu Base coordination could not {operation} (code {code}{request_id}): {msg}"
        ),
    }
}

fn paged_payload<'a>(data: &'a Value, label: &str) -> Result<&'a Value> {
    data.get("items")
        .is_some()
        .then_some(data)
        .or_else(|| data.get("data"))
        .with_context(|| format!("Feishu Base {label} response is missing items payload"))
}

fn default_instance_id(workspace_root: &str) -> String {
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());
    let mut hasher = DefaultHasher::new();
    workspace_root.hash(&mut hasher);
    format!("{hostname}-{}-{:x}", std::process::id(), hasher.finish())
}

fn heartbeat_key(app_id: &str, instance_id: &str) -> String {
    format!("{app_id}:{instance_id}")
}

fn force_key(app_id: &str) -> String {
    app_id.to_string()
}

fn tables_url(base_token: &str) -> String {
    format!("/open-apis/bitable/v1/apps/{base_token}/tables")
}

fn fields_url(base_token: &str, table_id: &str) -> String {
    format!("/open-apis/bitable/v1/apps/{base_token}/tables/{table_id}/fields")
}

fn records_url(base_token: &str, table_id: &str) -> String {
    format!("/open-apis/bitable/v1/apps/{base_token}/tables/{table_id}/records")
}

fn record_url(base_token: &str, table_id: &str, record_id: &str) -> String {
    format!("/open-apis/bitable/v1/apps/{base_token}/tables/{table_id}/records/{record_id}")
}

fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn field_type_name(field_type: i64) -> &'static str {
    match field_type {
        FIELD_TYPE_TEXT => "text",
        FIELD_TYPE_NUMBER => "number",
        _ => "unknown",
    }
}

fn string_field(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn integer_field(fields: &Map<String, Value>, key: &str) -> Option<i64> {
    let value = fields.get(key)?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_f64().map(|raw| raw as i64))
        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
}

fn duration_to_millis_i64(duration: Duration) -> Result<i64> {
    i64::try_from(duration.as_millis()).context("duration does not fit into i64 milliseconds")
}

fn unix_timestamp_ms_now() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(elapsed.as_millis()).context("unix timestamp milliseconds exceed i64")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::BaseTable;
    use super::FORCE_FIELDS;
    use super::ForceIntentRecord;
    use super::HEARTBEAT_FIELDS;
    use super::HeartbeatLease;
    use super::RequiredField;
    use super::choose_named_table;
    use super::classify_feishu_api_error;
    use super::select_leader;

    #[test]
    fn select_leader_prefers_higher_priority_then_instance_id() {
        let leader = select_leader(
            "instance_b",
            10_000,
            &[
                heartbeat("instance_b", "session_b", 200, 9_900, 500),
                heartbeat("instance_a", "session_a", 200, 9_900, 500),
                heartbeat("instance_c", "session_c", 100, 9_900, 500),
            ],
            None,
        )
        .expect("leader");

        assert_eq!(
            leader,
            super::LeadershipSnapshot {
                is_leader: false,
                leader_instance_id: "instance_a".to_string(),
                leader_session_id: "session_a".to_string(),
                forced_instance_id: None,
            }
        );
    }

    #[test]
    fn select_leader_honors_active_force_intent() {
        let leader = select_leader(
            "instance_b",
            10_000,
            &[
                heartbeat("instance_a", "session_a", 200, 9_900, 500),
                heartbeat("instance_b", "session_b", 100, 9_900, 500),
            ],
            Some(&force("instance_b", "session_b", 10_500, 9_999)),
        )
        .expect("leader");

        assert_eq!(
            leader,
            super::LeadershipSnapshot {
                is_leader: true,
                leader_instance_id: "instance_b".to_string(),
                leader_session_id: "session_b".to_string(),
                forced_instance_id: Some("instance_b".to_string()),
            }
        );
    }

    #[test]
    fn select_leader_ignores_expired_force_intent() {
        let leader = select_leader(
            "instance_b",
            10_000,
            &[
                heartbeat("instance_a", "session_a", 200, 9_900, 500),
                heartbeat("instance_b", "session_b", 100, 9_900, 500),
            ],
            Some(&force("instance_b", "session_b", 9_999, 9_900)),
        )
        .expect("leader");

        assert_eq!(leader.leader_instance_id, "instance_a");
        assert_eq!(leader.forced_instance_id, None);
    }

    #[test]
    fn select_leader_ignores_force_intent_when_target_is_inactive() {
        let leader = select_leader(
            "instance_b",
            10_000,
            &[
                heartbeat("instance_a", "session_a", 200, 9_900, 500),
                heartbeat("instance_b", "session_b", 100, 9_000, 500),
            ],
            Some(&force("instance_b", "session_b", 10_500, 9_999)),
        )
        .expect("leader");

        assert_eq!(leader.leader_instance_id, "instance_a");
        assert_eq!(leader.forced_instance_id, Some("instance_b".to_string()));
    }

    #[test]
    fn choose_named_table_is_deterministic_when_duplicates_exist() {
        let tables = vec![
            BaseTable {
                table_id: "tbl_b".to_string(),
                name: "clawbot_coordination_heartbeat".to_string(),
            },
            BaseTable {
                table_id: "tbl_a".to_string(),
                name: "clawbot_coordination_heartbeat".to_string(),
            },
        ];

        let chosen =
            choose_named_table(&tables, "clawbot_coordination_heartbeat").expect("named table");
        assert_eq!(chosen.table_id, "tbl_a");
    }

    #[test]
    fn required_field_sets_match_expected_shape() {
        assert_eq!(
            HEARTBEAT_FIELDS.first(),
            Some(&RequiredField::new("key", 1))
        );
        assert_eq!(FORCE_FIELDS.first(), Some(&RequiredField::new("key", 1)));
    }

    #[test]
    fn permission_error_is_actionable() {
        let response = open_lark::openlark_core::api::Response::error(
            1_254_302,
            "The role has no permissions.",
        );
        let message = classify_feishu_api_error(
            "create Feishu Base table clawbot_coordination_heartbeat",
            "bascn_test",
            &response,
        )
        .to_string();

        assert!(message.contains("Grant the app edit/admin access"));
        assert!(message.contains("bascn_test"));
    }

    fn heartbeat(
        instance_id: &str,
        session_id: &str,
        owner_priority: i64,
        last_seen_ms: i64,
        ttl_ms: i64,
    ) -> HeartbeatLease {
        HeartbeatLease {
            key: format!("app_test:{instance_id}"),
            app_id: "app_test".to_string(),
            instance_id: instance_id.to_string(),
            session_id: session_id.to_string(),
            owner_priority,
            last_seen_ms,
            ttl_ms,
            ws_state: "idle".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
        }
    }

    fn force(
        target_instance_id: &str,
        target_session_id: &str,
        force_until_ms: i64,
        requested_at_ms: i64,
    ) -> ForceIntentRecord {
        ForceIntentRecord {
            record_id: Some("rec_force".to_string()),
            key: "app_test".to_string(),
            app_id: "app_test".to_string(),
            target_instance_id: target_instance_id.to_string(),
            target_session_id: target_session_id.to_string(),
            force_until_ms,
            requested_at_ms,
        }
    }
}
