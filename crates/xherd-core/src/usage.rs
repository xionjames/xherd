//! Token usage and budgets (R-1205, pure).
//!
//! Usage is read from the runtime's transcript after each stop (the runtime
//! plugin declares how, `xherd_plugin::runtime::Usage`). The count is
//! cumulative per session, so a measurement that is skipped or dropped is
//! caught up by the next one.

use crate::{Agent, Millis};

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
        let mut a = Agent::new("A", 1);
        assert_eq!(a.record_usage("s1", 100), 100);
        assert_eq!(a.record_usage("s1", 250), 150);
        assert_eq!(a.record_usage("s1", 200), 0, "never goes backwards");
        assert_eq!(a.record_usage("s2", 40), 40, "a new session adds up");
        assert_eq!(a.tokens_total(), 290);
    }
}
