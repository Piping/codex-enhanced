use chrono::DateTime;
use chrono::TimeZone;
use chrono::Utc;
use cron::Schedule;
use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LoopSchedule {
    Interval { display: String, seconds: u64 },
    Cron { display: String, normalized: String },
}

impl LoopSchedule {
    pub(crate) fn display(&self) -> &str {
        match self {
            Self::Interval { display, .. } | Self::Cron { display, .. } => display,
        }
    }

    pub(crate) fn next_due_after(
        &self,
        last_scheduled_at_unix_seconds: i64,
        now: DateTime<Utc>,
    ) -> DateTime<Utc> {
        match self {
            Self::Interval { seconds, .. } => {
                let interval = i64::try_from(*seconds).unwrap_or(i64::MAX).max(1);
                let next = if last_scheduled_at_unix_seconds >= now.timestamp() {
                    last_scheduled_at_unix_seconds.saturating_add(interval)
                } else {
                    let elapsed = now
                        .timestamp()
                        .saturating_sub(last_scheduled_at_unix_seconds);
                    let skipped_intervals = elapsed / interval;
                    last_scheduled_at_unix_seconds.saturating_add(
                        skipped_intervals.saturating_add(1).saturating_mul(interval),
                    )
                };
                unix_seconds_to_utc(next).unwrap_or(now)
            }
            Self::Cron { normalized, .. } => Schedule::from_str(normalized)
                .ok()
                .and_then(|schedule| {
                    unix_seconds_to_utc(last_scheduled_at_unix_seconds)
                        .and_then(|last| schedule.after(&last).next())
                })
                .map(|next| next.with_timezone(&Utc))
                .filter(|next| *next > now)
                .unwrap_or(now),
        }
    }

    pub(crate) fn first_due_after_creation(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::Interval { seconds, .. } => {
                let interval = i64::try_from(*seconds).unwrap_or(i64::MAX).max(1);
                unix_seconds_to_utc(now.timestamp().saturating_add(interval)).unwrap_or(now)
            }
            Self::Cron { normalized, .. } => Schedule::from_str(normalized)
                .ok()
                .and_then(|schedule| schedule.after(&now).next())
                .map(|next| next.with_timezone(&Utc))
                .unwrap_or(now),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoopMode {
    OneShot,
    #[default]
    Persistent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LoopDeliveryMode {
    #[default]
    AssistantOnly,
    AssistantThenActionUser,
    ResultAsUser,
}

impl LoopDeliveryMode {
    pub(crate) const USER_SELECTABLE: [Self; 3] = [
        Self::AssistantOnly,
        Self::ResultAsUser,
        Self::AssistantThenActionUser,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::AssistantOnly => "As Assistant Message",
            Self::ResultAsUser => "As User Message",
            Self::AssistantThenActionUser => "As User Message + Action",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::AssistantOnly => {
                "Default. Mirror the latest loop result into the main thread as an assistant message."
            }
            Self::ResultAsUser => {
                "Submit the latest loop result back into the main thread as a user message."
            }
            Self::AssistantThenActionUser => {
                "Submit the latest loop result back into the main thread as a user message, with the configured action appended at the end."
            }
        }
    }

    pub(crate) fn short_label(self) -> &'static str {
        match self {
            Self::AssistantOnly => "assistant message",
            Self::ResultAsUser => "user message",
            Self::AssistantThenActionUser => "user message + action",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoopCommand {
    Focus {
        id: String,
    },
    Create {
        id: Option<String>,
        schedule: LoopSchedule,
        prompt: String,
    },
}

pub(crate) fn parse_loop_command(spec: &str) -> Result<LoopCommand, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(
            "Usage: /loop <duration|cron> <prompt> or /loop <id> <duration|cron> <prompt>"
                .to_string(),
        );
    }

    if let Ok((schedule, prompt)) = parse_schedule_and_prompt(spec) {
        if prompt.trim().is_empty() {
            return Err("expected a prompt after the schedule".to_string());
        }
        return Ok(LoopCommand::Create {
            id: None,
            schedule,
            prompt,
        });
    }

    let tokens = spec.split_whitespace().collect::<Vec<_>>();
    if tokens.len() == 1 {
        return Ok(LoopCommand::Focus {
            id: tokens[0].to_string(),
        });
    }

    let id = tokens[0].trim();
    validate_loop_id(id)?;
    let rest = spec[id.len()..].trim();
    let (schedule, prompt) = parse_schedule_and_prompt(rest)?;
    if prompt.trim().is_empty() {
        return Err("expected a prompt after the schedule".to_string());
    }
    Ok(LoopCommand::Create {
        id: Some(id.to_string()),
        schedule,
        prompt,
    })
}

pub(crate) fn parse_loop_schedule(spec: &str) -> Result<LoopSchedule, String> {
    let (schedule, prompt) = parse_schedule_and_prompt(spec)?;
    if !prompt.is_empty() {
        return Err("expected only a schedule".to_string());
    }
    Ok(schedule)
}

pub(crate) fn validate_loop_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("loop id cannot be empty".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("loop id must use only letters, digits, underscores, or hyphens".to_string());
    }
    Ok(())
}

fn parse_schedule_and_prompt(spec: &str) -> Result<(LoopSchedule, String), String> {
    let tokens = spec.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err("expected a schedule".to_string());
    }

    if let Some(seconds) = parse_interval_seconds(tokens[0]) {
        let prompt = spec[tokens[0].len()..].trim().to_string();
        return Ok((
            LoopSchedule::Interval {
                display: tokens[0].to_string(),
                seconds,
            },
            prompt,
        ));
    }

    for field_count in [7usize, 6, 5] {
        if tokens.len() < field_count {
            continue;
        }
        let display = tokens[..field_count].join(" ");
        let normalized = normalize_cron_expression(&display, field_count);
        if Schedule::from_str(&normalized).is_ok() {
            let prompt = tokens[field_count..].join(" ");
            return Ok((
                LoopSchedule::Cron {
                    display,
                    normalized,
                },
                prompt,
            ));
        }
    }

    Err(
        "could not parse the schedule; use `5m`-style intervals or a 5/6/7-field cron expression"
            .to_string(),
    )
}

fn parse_interval_seconds(token: &str) -> Option<u64> {
    let mut index = 0usize;
    let mut total = 0u64;
    let bytes = token.as_bytes();
    while index < bytes.len() {
        let digits_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if digits_start == index || index >= bytes.len() {
            return None;
        }
        let value = token[digits_start..index].parse::<u64>().ok()?;
        let multiplier = match bytes[index] as char {
            's' => 1,
            'm' => 60,
            'h' => 60 * 60,
            'd' => 60 * 60 * 24,
            _ => return None,
        };
        total = total.checked_add(value.checked_mul(multiplier)?)?;
        index += 1;
    }
    (total > 0).then_some(total)
}

fn normalize_cron_expression(expression: &str, field_count: usize) -> String {
    match field_count {
        5 => format!("0 {expression} *"),
        6 => format!("{expression} *"),
        _ => expression.to_string(),
    }
}

fn unix_seconds_to_utc(unix_seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(unix_seconds, 0).single()
}

#[cfg(test)]
mod tests {
    use super::LoopCommand;
    use super::LoopSchedule;
    use super::parse_loop_command;
    use super::parse_loop_schedule;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_loop_command_accepts_one_shot_interval() {
        let parsed = parse_loop_command("5m check status").expect("one-shot should parse");
        assert_eq!(
            parsed,
            LoopCommand::Create {
                id: None,
                schedule: LoopSchedule::Interval {
                    display: "5m".to_string(),
                    seconds: 300,
                },
                prompt: "check status".to_string(),
            }
        );
    }

    #[test]
    fn parse_loop_command_accepts_persistent_interval() {
        let parsed =
            parse_loop_command("director 30m review progress").expect("persistent should parse");
        assert_eq!(
            parsed,
            LoopCommand::Create {
                id: Some("director".to_string()),
                schedule: LoopSchedule::Interval {
                    display: "30m".to_string(),
                    seconds: 1800,
                },
                prompt: "review progress".to_string(),
            }
        );
    }

    #[test]
    fn parse_loop_command_accepts_focus_existing_loop() {
        let parsed = parse_loop_command("director").expect("focus should parse");
        assert_eq!(
            parsed,
            LoopCommand::Focus {
                id: "director".to_string()
            }
        );
    }

    #[test]
    fn parse_loop_schedule_accepts_cron_without_prompt() {
        let parsed = parse_loop_schedule("*/5 * * * *").expect("cron should parse");
        assert_eq!(
            parsed,
            LoopSchedule::Cron {
                display: "*/5 * * * *".to_string(),
                normalized: "0 */5 * * * * *".to_string(),
            }
        );
    }
}
