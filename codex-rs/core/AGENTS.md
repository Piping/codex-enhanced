# codex-rs/core

## codex-core Scope

`codex-core` is already large. Resist adding new code here when another crate or a new crate would
own the concept more cleanly.

- Before adding a new concept, feature, or API to `codex-core`, consider whether an existing crate
  other than `codex-core` is the better home.
- If no existing crate fits, consider introducing a new crate to the workspace instead of growing
  `codex-core`.
- When reviewing changes, push back on additions that grow `codex-core` without a clear reason.

## Integration Tests

- Prefer the utilities in `core_test_support::responses` when writing end-to-end Codex tests.
- Hold on to the `ResponseMock` returned by `mount_sse*` helpers so assertions can inspect outbound
  `/responses` POST bodies.
- Use `ResponseMock::single_request()` when a test should issue one POST. Use
  `ResponseMock::requests()` when you need the full capture set.
- Prefer the structured request helpers, such as `body_json`, `input`, `function_call_output`,
  `custom_tool_call_output`, `call_output`, `header`, `path`, and `query_param`, over manual JSON
  digging.
- Build SSE payloads with the provided `ev_*` constructors and `sse(...)`.
- Prefer `wait_for_event` over `wait_for_event_with_timeout`.
- Prefer `mount_sse_once` over `mount_sse_once_match` or `mount_sse_sequence`.
