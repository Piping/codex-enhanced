use std::path::Path;

use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use pathdiff::diff_paths;

use super::aggregator::format_duration;
use super::aggregator::format_number;
use super::types::InsightReportData;
use super::types::NarrativeMode;
use super::types::RootSessionSummary;

pub(crate) async fn write_report_html(data: &InsightReportData) -> anyhow::Result<()> {
    let parent_dir = data
        .report_path
        .parent()
        .context("report path has no parent directory")?;
    tokio::fs::create_dir_all(parent_dir)
        .await
        .with_context(|| format!("failed to create {}", parent_dir.display()))?;
    let html = render_html(data);
    tokio::fs::write(&data.report_path, html)
        .await
        .with_context(|| format!("failed to write {}", data.report_path.display()))?;
    Ok(())
}

pub(crate) fn render_html(data: &InsightReportData) -> String {
    let generated_at = format_datetime(data.generated_at);
    let earliest = data
        .overview
        .earliest_event_at
        .map(format_datetime)
        .unwrap_or_else(|| "N/A".to_string());
    let latest = data
        .overview
        .latest_event_at
        .map(format_datetime)
        .unwrap_or_else(|| "N/A".to_string());
    let narrative_mode = match data.narrative_mode {
        NarrativeMode::LocalHeuristics => "Local heuristics fallback / 本地启发式回退",
    };

    let top_roots_rows = data
        .roots
        .iter()
        .take(10)
        .map(|root| {
            format!(
                "<tr><td><a href=\"#root-{id}\">{title}</a></td><td>{threads}</td><td>{tokens}</td><td>{wall}</td><td>{failures}</td></tr>",
                id = escape_html(root.root_thread_id.to_string().as_str()),
                title = escape_html(root.title.as_str()),
                threads = root.threads.len(),
                tokens = format_number(root.metrics.total_tokens),
                wall = format_duration(root.wall_clock_span),
                failures = root.metrics.total_failures(),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let root_sections = data
        .roots
        .iter()
        .map(|root| render_root_section(root, data.codex_home.as_path()))
        .collect::<Vec<_>>()
        .join("");

    let patterns = render_list(data.common_patterns.iter().map(std::string::String::as_str));
    let suggestions = render_list(
        data.efficiency_suggestions
            .iter()
            .map(std::string::String::as_str),
    );

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Codex Insight Report</title>
  <style>
    :root {{
      --bg: #f7f3ea;
      --panel: #fffdf8;
      --ink: #1d1a17;
      --muted: #6d655d;
      --line: #ded4c6;
      --accent: #b44c2f;
      --accent-soft: #f2dfd8;
      --shadow: rgba(36, 24, 14, 0.08);
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      font-family: "Iowan Old Style", "Palatino Linotype", "Noto Serif SC", serif;
      color: var(--ink);
      background:
        radial-gradient(circle at top left, rgba(180, 76, 47, 0.10), transparent 32%),
        linear-gradient(180deg, #f3ecde 0%, var(--bg) 38%, #f9f6ef 100%);
      line-height: 1.55;
    }}
    main {{
      max-width: 1180px;
      margin: 0 auto;
      padding: 32px 20px 64px;
    }}
    header {{
      padding: 28px;
      border: 1px solid var(--line);
      border-radius: 24px;
      background: linear-gradient(135deg, rgba(255,255,255,0.92), rgba(255,247,240,0.98));
      box-shadow: 0 14px 40px var(--shadow);
    }}
    h1, h2, h3 {{ margin: 0 0 12px; }}
    h1 {{ font-size: clamp(2rem, 4vw, 3.2rem); }}
    h2 {{ margin-top: 36px; font-size: 1.45rem; }}
    .muted {{ color: var(--muted); }}
    .toc {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 18px;
    }}
    .toc a {{
      color: var(--accent);
      text-decoration: none;
      padding: 8px 12px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: rgba(255,255,255,0.7);
    }}
    .cards {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 14px;
      margin-top: 22px;
    }}
    .card {{
      padding: 16px;
      border: 1px solid var(--line);
      border-radius: 18px;
      background: var(--panel);
      box-shadow: 0 8px 24px var(--shadow);
    }}
    .card strong {{
      display: block;
      font-size: 1.45rem;
      margin-top: 6px;
    }}
    section {{
      margin-top: 28px;
      padding: 24px;
      border: 1px solid var(--line);
      border-radius: 22px;
      background: rgba(255, 253, 248, 0.92);
      box-shadow: 0 10px 30px var(--shadow);
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      margin-top: 14px;
      font-size: 0.95rem;
    }}
    th, td {{
      text-align: left;
      padding: 10px 12px;
      border-bottom: 1px solid var(--line);
      vertical-align: top;
    }}
    th {{
      color: var(--muted);
      font-weight: 700;
    }}
    code {{
      font-family: "SFMono-Regular", "Menlo", monospace;
      font-size: 0.92em;
      background: #f4eee4;
      padding: 2px 6px;
      border-radius: 6px;
    }}
    ul {{ padding-left: 20px; }}
    .pill {{
      display: inline-block;
      padding: 4px 10px;
      border-radius: 999px;
      background: var(--accent-soft);
      color: var(--accent);
      font-size: 0.86rem;
      margin-right: 8px;
    }}
    .metric-strip {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 12px;
    }}
    .metric-strip span {{
      border: 1px solid var(--line);
      border-radius: 999px;
      padding: 7px 12px;
      background: #fff;
    }}
    .root {{
      scroll-margin-top: 20px;
    }}
    .small {{ font-size: 0.9rem; }}
    @media (max-width: 720px) {{
      main {{ padding: 18px 14px 40px; }}
      header, section {{ padding: 18px; border-radius: 18px; }}
      table, thead, tbody, tr, th, td {{ display: block; }}
      thead {{ display: none; }}
      td {{
        padding: 8px 0;
        border-bottom: none;
      }}
      tr {{
        padding: 12px 0;
        border-bottom: 1px solid var(--line);
      }}
    }}
  </style>
</head>
<body>
  <main>
    <header>
      <span class="pill">/insight</span>
      <span class="pill">{narrative_mode}</span>
      <h1>Codex Insight Report</h1>
      <p class="muted">Dashboard first, drill-down later. Mixed CN/EN report. Generated at {generated_at}.</p>
      <p class="muted small">History window: {earliest} → {latest}</p>
      <div class="toc">
        <a href="#summary">Executive Summary</a>
        <a href="#top-sessions">Top Sessions</a>
        <a href="#token-analysis">Token Analysis</a>
        <a href="#time-analysis">Time Analysis</a>
        <a href="#failure-analysis">Failure Analysis</a>
        <a href="#tool-analysis">Tool / Patch Analysis</a>
        <a href="#patterns">Common Patterns</a>
        <a href="#suggestions">Efficiency Suggestions</a>
        <a href="#drilldown">Root Session Drill-down</a>
        <a href="#methodology">Methodology</a>
      </div>
      <div class="cards">
        <div class="card"><span class="muted">Root Sessions</span><strong>{root_sessions}</strong></div>
        <div class="card"><span class="muted">Threads</span><strong>{threads}</strong></div>
        <div class="card"><span class="muted">Tokens</span><strong>{tokens}</strong></div>
        <div class="card"><span class="muted">Exact Tool Runtime</span><strong>{tool_runtime}</strong></div>
        <div class="card"><span class="muted">Failure Rate</span><strong>{failure_rate:.1}%</strong></div>
        <div class="card"><span class="muted">History Span</span><strong>{history_span}</strong></div>
      </div>
    </header>

    <section id="summary">
      <h2>Executive Summary / 概览</h2>
      <p>Total tokens: <strong>{tokens}</strong>. Counted operations: <strong>{counted_operations}</strong>. Failures: <strong>{failures}</strong>.</p>
      <p class="muted">Scanned {scanned_files} rollout file(s); skipped {skipped_files}. Archived thread(s): {archived_threads}.</p>
    </section>

    <section id="top-sessions">
      <h2>Top Sessions / 高消耗会话</h2>
      <table>
        <thead>
          <tr><th>Root Session</th><th>Threads</th><th>Tokens</th><th>Wall Span</th><th>Failures</th></tr>
        </thead>
        <tbody>{top_roots_rows}</tbody>
      </table>
    </section>

    <section id="token-analysis">
      <h2>Token Analysis / Token 分析</h2>
      <div class="metric-strip">
        <span>Total: {tokens}</span>
        <span>Input: {input_tokens}</span>
        <span>Output: {output_tokens}</span>
        <span>Reasoning Output: {reasoning_output_tokens}</span>
      </div>
    </section>

    <section id="time-analysis">
      <h2>Time Analysis / 时间分析</h2>
      <div class="metric-strip">
        <span>History Span: {history_span}</span>
        <span>Cumulative Thread Span: {cumulative_thread_span}</span>
        <span>Exact Tool Runtime: {tool_runtime}</span>
        <span>Estimated User Wait: {estimated_user_wait}</span>
        <span>Residual Model/UI Time: {residual_runtime}</span>
      </div>
      <p class="muted small">Residual time is a conservative estimate after subtracting exact persisted tool durations and estimated user idle gaps.</p>
    </section>

    <section id="failure-analysis">
      <h2>Failure Analysis / 失败分析</h2>
      <div class="metric-strip">
        <span>Exec failures: {exec_failures}</span>
        <span>MCP failures: {mcp_failures}</span>
        <span>Dynamic tool failures: {dynamic_failures}</span>
        <span>Patch failures: {patch_failures}</span>
        <span>API errors: {api_errors}</span>
      </div>
    </section>

    <section id="tool-analysis">
      <h2>Tool / Patch Analysis / 工具与补丁</h2>
      <div class="metric-strip">
        <span>Exec commands: {exec_count}</span>
        <span>Tool calls: {tool_calls}</span>
        <span>Patches: {patch_count}</span>
        <span>Patched files: {patched_files}</span>
        <span>User messages: {user_messages}</span>
        <span>Completed turns: {completed_turns}</span>
      </div>
    </section>

    <section id="patterns">
      <h2>Common Patterns / 共性模式</h2>
      {patterns}
    </section>

    <section id="suggestions">
      <h2>Efficiency Suggestions / 效率建议</h2>
      {suggestions}
    </section>

    <section id="drilldown">
      <h2>Root Session Drill-down / 根会话下钻</h2>
      {root_sections}
    </section>

    <section id="methodology">
      <h2>Methodology / 指标说明</h2>
      <ul>
        <li><strong>Exact</strong>: exec command, MCP tool, and dynamic tool durations come from persisted rollout events.</li>
        <li><strong>Exact</strong>: wall-clock spans come from persisted rollout timestamps.</li>
        <li><strong>Estimated</strong>: user wait is measured as the gap from a completed turn to the next user message when that gap is observable in history.</li>
        <li><strong>Estimated</strong>: residual model/UI time is the remaining thread span after subtracting exact tool runtime and estimated user wait.</li>
        <li><strong>Failure rate</strong>: failures / counted operations, where counted operations are exec + MCP + dynamic tools + patch applications + API error incidents.</li>
        <li><strong>Narrative layer</strong>: this report used {narrative_mode}, so pattern and suggestion sections are deterministic local heuristics instead of a model-generated write-up.</li>
      </ul>
    </section>
  </main>
</body>
</html>"##,
        narrative_mode = escape_html(narrative_mode),
        generated_at = escape_html(generated_at.as_str()),
        root_sessions = data.overview.total_root_sessions,
        threads = data.overview.total_threads,
        tokens = format_number(data.overview.metrics.total_tokens),
        tool_runtime = format_duration(data.overview.metrics.exact_tool_runtime()),
        failure_rate = data.overview.metrics.failure_rate() * 100.0,
        history_span = format_duration(data.overview.history_span),
        counted_operations = data.overview.metrics.counted_operations(),
        failures = data.overview.metrics.total_failures(),
        scanned_files = data.overview.scanned_files,
        skipped_files = data.overview.skipped_files,
        archived_threads = data.overview.archived_threads,
        input_tokens = format_number(data.overview.metrics.input_tokens),
        output_tokens = format_number(data.overview.metrics.output_tokens),
        reasoning_output_tokens = format_number(data.overview.metrics.reasoning_output_tokens),
        cumulative_thread_span = format_duration(data.overview.metrics.cumulative_thread_span),
        estimated_user_wait = format_duration(data.overview.metrics.estimated_user_wait),
        residual_runtime = format_duration(data.overview.metrics.residual_runtime_estimate),
        exec_failures = data.overview.metrics.exec_commands.failures,
        mcp_failures = data.overview.metrics.mcp_tool_calls.failures,
        dynamic_failures = data.overview.metrics.dynamic_tool_calls.failures,
        patch_failures = data.overview.metrics.patches.failures,
        api_errors = data.overview.metrics.api_error_count,
        exec_count = data.overview.metrics.exec_commands.count,
        tool_calls = data.overview.metrics.tool_call_count(),
        patch_count = data.overview.metrics.patches.count,
        patched_files = data.overview.metrics.patches.changed_files,
        user_messages = data.overview.metrics.user_message_count,
        completed_turns = data.overview.metrics.completed_turn_count,
        top_roots_rows = top_roots_rows,
        patterns = patterns,
        suggestions = suggestions,
        root_sections = root_sections,
    )
}

fn render_root_section(root: &RootSessionSummary, codex_home: &Path) -> String {
    let relative_rollout =
        diff_paths(&root.rollout_path, codex_home).unwrap_or_else(|| root.rollout_path.clone());
    let cwd = if root.cwd.as_os_str().is_empty() {
        "N/A".to_string()
    } else {
        escape_html(root.cwd.display().to_string().as_str())
    };
    let started_at = root
        .earliest_event_at
        .map(format_datetime)
        .unwrap_or_else(|| "N/A".to_string());
    let latest_at = root
        .latest_event_at
        .map(format_datetime)
        .unwrap_or_else(|| "N/A".to_string());
    let rows = root
        .threads
        .iter()
        .map(|thread| {
            let thread_rollout = diff_paths(&thread.rollout_path, codex_home)
                .unwrap_or_else(|| thread.rollout_path.clone());
            format!(
                "<tr><td>{title}</td><td>{source}</td><td>{tokens}</td><td>{wall}</td><td>{tools}</td><td>{patches}</td><td><code>{rollout}</code></td></tr>",
                title = escape_html(thread.title.as_str()),
                source = escape_html(thread.source_label.as_str()),
                tokens = format_number(thread.metrics.total_tokens),
                wall = format_duration(thread.wall_clock_span()),
                tools = thread.metrics.tool_call_count(),
                patches = thread.metrics.patches.count,
                rollout = escape_html(thread_rollout.display().to_string().as_str()),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<section class="root" id="root-{id}">
  <h3>{title}</h3>
  <p class="muted small">Root thread: <code>{id}</code> · Rollout: <code>{rollout}</code> · CWD: <code>{cwd}</code></p>
  <div class="metric-strip">
    <span>Threads: {thread_count}</span>
    <span>Tokens: {tokens}</span>
    <span>Wall Span: {wall_span}</span>
    <span>Exact Tool Runtime: {tool_runtime}</span>
    <span>Failure Rate: {failure_rate:.1}%</span>
  </div>
  <p class="muted small">Timeline: {started_at} → {latest_at}</p>
  <table>
    <thead>
      <tr><th>Thread</th><th>Source</th><th>Tokens</th><th>Wall Span</th><th>Tool Calls</th><th>Patches</th><th>Rollout</th></tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>
</section>"#,
        id = escape_html(root.root_thread_id.to_string().as_str()),
        title = escape_html(root.title.as_str()),
        rollout = escape_html(relative_rollout.display().to_string().as_str()),
        cwd = cwd,
        thread_count = root.threads.len(),
        tokens = format_number(root.metrics.total_tokens),
        wall_span = format_duration(root.wall_clock_span),
        tool_runtime = format_duration(root.metrics.exact_tool_runtime()),
        failure_rate = root.metrics.failure_rate() * 100.0,
        started_at = escape_html(started_at.as_str()),
        latest_at = escape_html(latest_at.as_str()),
        rows = rows,
    )
}

fn render_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let items = items
        .map(|item| format!("<li>{}</li>", escape_html(item)))
        .collect::<Vec<_>>()
        .join("");
    format!("<ul>{items}</ul>")
}

fn format_datetime(datetime: DateTime<Utc>) -> String {
    datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use chrono::TimeZone;
    use chrono::Utc;
    use insta::assert_snapshot;

    use super::render_html;
    use crate::insight::types::AggregateMetrics;
    use crate::insight::types::InsightOverview;
    use crate::insight::types::InsightReportData;
    use crate::insight::types::NarrativeMode;
    use crate::insight::types::RootSessionSummary;
    use codex_protocol::ThreadId;

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::from_string(value).expect("valid thread id")
    }

    #[test]
    fn report_html_snapshot() {
        let generated_at = Utc.with_ymd_and_hms(2026, 4, 8, 12, 30, 0).unwrap();
        let metrics = AggregateMetrics {
            total_tokens: 12345,
            cumulative_thread_span: Duration::from_secs(600),
            estimated_user_wait: Duration::from_secs(120),
            residual_runtime_estimate: Duration::from_secs(180),
            ..AggregateMetrics::default()
        };
        let root = RootSessionSummary {
            root_thread_id: thread_id("00000000-0000-0000-0000-000000000001"),
            title: "Investigate /insight".to_string(),
            cwd: PathBuf::from("/repo"),
            rollout_path: PathBuf::from("/tmp/codex-home/sessions/root.jsonl"),
            archived: false,
            earliest_event_at: Some(generated_at),
            latest_event_at: Some(generated_at + chrono::Duration::seconds(600)),
            wall_clock_span: Duration::from_secs(600),
            metrics: metrics.clone(),
            threads: Vec::new(),
        };
        let html = render_html(&InsightReportData {
            generated_at,
            codex_home: PathBuf::from("/tmp/codex-home"),
            report_path: PathBuf::from("/tmp/codex-home/reports/insight-20260408-123000.html"),
            overview: InsightOverview {
                total_root_sessions: 1,
                total_threads: 1,
                archived_threads: 0,
                scanned_files: 1,
                skipped_files: 0,
                metrics,
                earliest_event_at: Some(generated_at),
                latest_event_at: Some(generated_at + chrono::Duration::seconds(600)),
                history_span: Duration::from_secs(600),
            },
            roots: vec![root],
            common_patterns: vec!["Pattern A".to_string()],
            efficiency_suggestions: vec!["Suggestion A".to_string()],
            narrative_mode: NarrativeMode::LocalHeuristics,
        });
        assert_snapshot!("insight_report_html", html);
    }
}
