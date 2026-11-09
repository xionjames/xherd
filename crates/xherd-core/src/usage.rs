//! Token usage and budgets (R-1205, pure).
//!
//! Usage is read from the runtime's transcript after each stop. The count is
//! cumulative per session, so a measurement that is skipped or dropped is
//! caught up by the next one.

use std::collections::HashMap;

use serde_json::Value;

use crate::{Agent, Millis};

/// Tokens of a Claude Code transcript (JSON lines): input + output + cache
/// creation of every assistant message. Cache reads are left out: they repeat
/// the whole context on each turn and would swamp a budget. The runtime writes
/// a message once per content block with the same `message.id`, so the last
/// line of each id counts. Lines that are not JSON are skipped.
pub fn transcript_tokens(jsonl: &str) -> u64 {
    let mut by_id: HashMap<String, u64> = HashMap::new();
    let mut anonymous = 0u64;
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(u) = v.pointer("/message/usage") else {
            continue;
        };
        let n = [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .filter_map(|k| u.get(k).and_then(Value::as_u64))
        .sum::<u64>();
        match v.pointer("/message/id").and_then(Value::as_str) {
            Some(id) => {
                by_id.insert(id.to_string(), n);
            }
            None => anonymous += n,
        }
    }
    by_id.values().sum::<u64>() + anonymous
}

/// `YYYY-MM-DD` (UTC) of a time in ms.
pub fn day(ms: Millis) -> String {
    // Days since 1970-01-01 to a civil date (H. Hinnant's algorithm).
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// No budget set, or well below it.
    Fine,
    /// At 80 % or more.
    Warn,
    /// At 100 % or more.
    Exceeded,
}

/// `budget` 0 = no budget.
pub fn level(used: u64, budget: u64) -> Level {
    if budget == 0 {
        Level::Fine
    } else if used >= budget {
        Level::Exceeded
    } else if used.saturating_mul(10) >= budget.saturating_mul(8) {
        Level::Warn
    } else {
        Level::Fine
    }
}

impl Agent {
    /// All tokens this agent used so far.
    pub fn tokens_total(&self) -> u64 {
        self.tokens_base + self.tokens_session
    }

    /// Record a measurement of session `id` (cumulative for that session).
    /// Returns the tokens added since the last one.
    pub fn record_usage(&mut self, id: &str, tokens: u64) -> u64 {
        if self.tokens_session_id.as_deref() == Some(id) {
            let delta = tokens.saturating_sub(self.tokens_session);
            self.tokens_session = self.tokens_session.max(tokens);
            delta
        } else {
            self.tokens_base += self.tokens_session;
            self.tokens_session_id = Some(id.to_string());
            self.tokens_session = tokens;
            tokens
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_message_once_and_skips_cache_reads() {
        let t = r#"{"type":"user","message":{"content":"hi"}}
{"message":{"id":"m1","usage":{"input_tokens":2,"output_tokens":5,"cache_creation_input_tokens":10,"cache_read_input_tokens":9000}}}
{"message":{"id":"m1","usage":{"input_tokens":2,"output_tokens":40,"cache_creation_input_tokens":10,"cache_read_input_tokens":9000}}}
not json
{"message":{"id":"m2","usage":{"input_tokens":1,"output_tokens":1}}}
{"message":{"usage":{"output_tokens":7}}}"#;
        // m1: 2+40+10, m2: 2, no id: 7
        assert_eq!(transcript_tokens(t), 52 + 2 + 7);
        assert_eq!(transcript_tokens(""), 0);
    }

    #[test]
    fn days_are_utc_dates() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(1_709_164_800_000), "2024-02-29");
        assert_eq!(day(1_709_251_199_999), "2024-02-29");
        assert_eq!(day(1_709_251_200_000), "2024-03-01");
        assert_eq!(day(-1), "1969-12-31");
    }

    #[test]
    fn levels() {
        assert_eq!(level(1_000_000, 0), Level::Fine);
        assert_eq!(level(79, 100), Level::Fine);
        assert_eq!(level(80, 100), Level::Warn);
        assert_eq!(level(100, 100), Level::Exceeded);
    }

    #[test]
    fn usage_is_cumulative_per_session() {
        let mut a = Agent::new("A", "A", 1);
        assert_eq!(a.record_usage("s1", 100), 100);
        assert_eq!(a.record_usage("s1", 250), 150);
        assert_eq!(a.record_usage("s1", 200), 0, "never goes backwards");
        assert_eq!(a.record_usage("s2", 40), 40, "a new session adds up");
        assert_eq!(a.tokens_total(), 290);
    }
}
