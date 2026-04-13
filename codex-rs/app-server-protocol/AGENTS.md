# codex-rs/app-server-protocol

These guidelines apply to app-server protocol work, especially in `src/protocol/common.rs`,
`src/protocol/v2.rs`, and the related docs in `../app-server/README.md`.

## Core Rules

- All active API development should happen in app-server v2. Do not add new API surface area to v1.
- Follow payload naming consistently:
  - `*Params` for request payloads
  - `*Response` for responses
  - `*Notification` for notifications
- Expose RPC methods as `<resource>/<method>` and keep `<resource>` singular, for example
  `thread/read` and `app/list`.
- Expose fields as camelCase on the wire with `#[serde(rename_all = "camelCase")]` unless a tagged
  union or explicit compatibility requirement needs a targeted rename.
- Config RPC payloads are the exception and should stay snake_case to mirror `config.toml` keys.
- Always set `#[ts(export_to = "v2/")]` on v2 request, response, and notification types.
- Never use `#[serde(skip_serializing_if = "Option::is_none")]` for v2 API payload fields.
  Exception: client to server requests that intentionally have no params may use
  `params: #[ts(type = "undefined")] #[serde(skip_serializing_if = "Option::is_none")] Option<()>`.
- Keep Rust and TypeScript wire renames aligned. If a field or variant uses `#[serde(rename = "...")]`,
  add the matching `#[ts(rename = "...")]`.
- For discriminated unions, use explicit tagging in both serializers:
  `#[serde(tag = "type", ...)]` and `#[ts(tag = "type", ...)]`.
- Prefer plain `String` IDs at the API boundary. Do UUID parsing or conversion internally.
- Timestamps should be integer Unix seconds (`i64`) and named `*_at`, such as `created_at`,
  `updated_at`, or `resets_at`.
- For experimental API surface area, use `#[experimental("method/or/field")]`, derive
  `ExperimentalApi` when field-level gating is needed, and use `inspect_params: true` in
  `common.rs` when only some fields of a method are experimental.

## Client To Server Params

- Every optional field in `*Params` must be annotated with `#[ts(optional = nullable)]`.
- Do not use `#[ts(optional = nullable)]` outside client to server request payloads.
- Optional collection fields should use `Option<...>` plus `#[ts(optional = nullable)]`.
- Do not use `#[serde(default)]` to model optional collections, and do not use
  `skip_serializing_if` on v2 payload fields.
- When omission should mean `false` for a boolean field, prefer
  `#[serde(default, skip_serializing_if = "std::ops::Not::not")] pub field: bool` over
  `Option<bool>`.
- New list methods should implement cursor pagination by default with request fields
  `cursor: Option<String>` and `limit: Option<u32>`, and response fields `data: Vec<...>` and
  `next_cursor: Option<String>`.

## Workflow

- Update docs and examples when API behavior changes, at minimum `../app-server/README.md`.
- Regenerate schema fixtures when API shapes change with `just write-app-server-schema`, and also
  run `just write-app-server-schema --experimental` when experimental fixtures are affected.
- Keep protocol-specific tests and broader validation in the release or tag flow unless the user
  explicitly asks to run them earlier.
