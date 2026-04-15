# Clawbot Feishu Base Coordination

`codex-clawbot` can use Feishu Base as a free coordination backend for embedded websocket
ownership. Every Codex process participates in leader election, but only the elected leader for a
given Feishu `app_id` should hold the websocket connection.

## Ownership

The runtime data should be owned and maintained by the Feishu app itself:

- The Feishu app credentials (`app_id` / `app_secret`) write heartbeat and force-intent rows.
- `lark-cli` is only a bootstrap and repair tool for creating the Base, inspecting rows, or fixing
  schema drift.
- Do not make `lark-cli` a runtime dependency for normal websocket ownership.

## Config

Add the coordination block under `.codex/clawbot/config.toml`:

```toml
[feishu]
app_id = "cli_xxx"
app_secret = "xxx"

[feishu.coordination]
base_token = "bascnxxxx"
heartbeat_table_id = "tblHeartbeat"
force_table_id = "tblForce"
owner_priority = 100
force_connect = false
# Optional. When empty, codex-clawbot auto-generates a per-process instance id.
# instance_id = "machine-a-codex-1"
```

`force_connect = true` means the current Codex process continuously refreshes a force-intent row
for its own `app_id`, so it preempts other contenders as soon as they observe the update.

## Base Schema

Create one Base with two tables. Use ASCII field names exactly as listed here.

### `heartbeat`

Required fields:

- `key` (text, primary field): `{app_id}:{instance_id}`
- `app_id` (text)
- `instance_id` (text)
- `session_id` (text)
- `owner_priority` (number)
- `last_seen_ms` (number)
- `ttl_ms` (number)
- `ws_state` (text)
- `workspace_root` (text)

Each Codex process owns one heartbeat row per `app_id + instance_id` and updates it on every
coordination tick.

### `force`

Required fields:

- `key` (text, primary field): `{app_id}`
- `app_id` (text)
- `target_instance_id` (text)
- `target_session_id` (text)
- `force_until_ms` (number)
- `requested_at_ms` (number)

There should be at most one logical active force row per `app_id`. The runtime treats the newest
row as authoritative.

## Election Rule

Leader selection is deterministic:

1. Keep only heartbeat rows whose `last_seen_ms + ttl_ms >= now`.
2. If an active force row exists and its target instance still has an active heartbeat, that target
   wins.
3. Otherwise choose the highest `owner_priority`.
4. Break ties by `instance_id` ascending, then `session_id` ascending.

When a process loses leadership, it should stop owning the websocket immediately and continue
publishing only heartbeat state until it becomes leader again.
