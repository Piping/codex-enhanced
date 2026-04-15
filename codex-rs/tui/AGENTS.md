# codex-rs/tui

See `styles.md` for the broader TUI style conventions.

- Use concise styling helpers from ratatui's `Stylize` trait.
  - Basic spans: use `"text".into()`.
  - Styled spans: use `"text".red()`, `"text".green()`, `"text".magenta()`, `"text".dim()`, and
    similar helpers.
  - Prefer these over constructing styles with `Span::styled` and `Style` directly.
- Prefer Stylize helpers such as `.dim()`, `.bold()`, `.cyan()`, `.italic()`, and
  `.underlined()` instead of manual `Style` construction where possible.
- Prefer simple conversions: use `"text".into()` for spans and `vec![...].into()` for lines. When
  inference is ambiguous, use `Line::from(...)` or `Span::from(...)`.
- If the style is computed at runtime, `Span::styled` is acceptable.
- Avoid hardcoded white. Prefer the default foreground instead of `.white()`.
- Chain style helpers for readability when it keeps the code clear, such as
  `url.cyan().underlined()`.
- Avoid churn between equivalent forms like `Span::styled` and `set_style`, or `Line::from(...)`
  and `.into()`, unless one form is clearly more readable.
- Prefer the form that stays compact after `rustfmt`.

## Text Wrapping

- Always use `textwrap::wrap` to wrap plain strings.
- If you need to wrap a ratatui `Line`, use the helpers in `tui/src/wrapping.rs`, such as
  `word_wrap_lines` or `word_wrap_line`.
- If you need to indent wrapped lines, prefer `RtOptions` `initial_indent` and
  `subsequent_indent` over custom logic.
- If you need to prefix a list of lines, use the `prefix_lines` helper from `line_utils`.

## Snapshot Coverage

- Any change that affects user-visible UI should keep snapshot coverage up to date by adding or
  updating the relevant `insta` snapshot tests.
- Generating, reviewing, and accepting snapshot updates belongs in the release or tag flow unless
  the user explicitly asks to do it during local iteration.
- For TUI PTY validation in this repo, prefer running with `-p newapi` so startup is not blocked by onboarding or auth flows.
