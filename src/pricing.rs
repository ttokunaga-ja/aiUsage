use crate::model::{Event, Provider};
use serde::Deserialize;
use std::collections::BTreeMap;

// Integer nano-USD per million tokens: a token's cost unit is 10^-15 USD.
#[derive(Deserialize)]
struct ClaudeRates {
    models: BTreeMap<String, [u64; 5]>,
}
#[derive(Deserialize)]
struct Schedule {
    short: [Option<u64>; 4],
    long: [Option<u64>; 4],
}
#[derive(Deserialize)]
struct OpenaiRates {
    long_input_threshold: u64,
    models: BTreeMap<String, Schedule>,
}
pub struct Pricing {
    claude: ClaudeRates,
    openai: OpenaiRates,
}

impl Pricing {
    pub fn new() -> Self {
        Self {
            claude: serde_json::from_str(include_str!("../data/claude-pricing.json"))
                .expect("bundled Claude pricing"),
            openai: serde_json::from_str(include_str!("../data/openai-pricing.json"))
                .expect("bundled OpenAI pricing"),
        }
    }
    pub fn cost(&self, provider: Provider, event: &Event) -> Option<u128> {
        let t = &event.tokens;
        match provider {
            Provider::Claude => {
                let model = strip_date(&event.model);
                let r = self.claude.models.get(model)?;
                if t.write != t.write_5m.checked_add(t.write_1h)? {
                    return None;
                }
                if event.fast
                    && !["claude-opus-4-8", "claude-opus-5", "claude-opus-5-5"].contains(&model)
                {
                    return None;
                }
                let counts = [t.input, t.read, t.write_5m, t.write_1h, t.output];
                let mut cost: u128 = counts
                    .iter()
                    .zip(r)
                    .map(|(c, r)| u128::from(*c) * u128::from(*r))
                    .sum();
                if event.fast {
                    cost *= 2;
                }
                if event.us_geo {
                    cost = cost * 11 / 10;
                }
                Some(cost)
            }
            Provider::Chatgpt => {
                if event.provider != "openai" {
                    return None;
                }
                let schedule = self
                    .openai
                    .models
                    .get(&event.model)
                    .or_else(|| self.openai.models.get(strip_date(&event.model)))?;
                let r = if event.context_input > self.openai.long_input_threshold {
                    // Null entire long schedule means no separate long tariff.
                    if schedule.long.iter().all(Option::is_none) {
                        &schedule.short
                    } else {
                        &schedule.long
                    }
                } else {
                    &schedule.short
                };
                let counts = [t.input, t.read, t.write, t.output];
                let mut cost = 0;
                for (count, rate) in counts.iter().zip(r) {
                    if *count == 0 {
                        continue;
                    }
                    cost += u128::from(*count) * u128::from((*rate)?);
                }
                Some(cost)
            }
        }
    }
}

fn strip_date(model: &str) -> &str {
    if let Some((prefix, suffix)) = model.rsplit_once('-')
        && suffix.len() == 8
        && suffix.bytes().all(|b| b.is_ascii_digit())
    {
        return prefix;
    }
    model
}

pub fn usd(cost: u128) -> String {
    const SCALE: u128 = 1_000_000_000_000_000;
    let fraction = format!("{:015}", cost % SCALE);
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{}.{}",
        cost / SCALE,
        if fraction.is_empty() { "0" } else { fraction }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tokens;
    fn event(model: &str, tokens: Tokens, context_input: u64) -> Event {
        Event {
            timestamp: "2026-09-01T00:00:00Z".parse().unwrap(),
            model: model.into(),
            provider: "openai".into(),
            tokens,
            fast: false,
            us_geo: false,
            context_input,
        }
    }
    #[test]
    fn cache_is_priced_separately_and_reasoning_is_not_added() {
        let e = event(
            "gpt-6.1-sol",
            Tokens {
                input: 70,
                read: 20,
                write: 10,
                output: 5,
                ..Tokens::default()
            },
            100,
        );
        assert_eq!(
            usd(Pricing::new().cost(Provider::Chatgpt, &e).unwrap()),
            "0.000217"
        );
        let e = event("gpt-6.1-sol", e.tokens, 272001);
        assert_eq!(
            usd(Pricing::new().cost(Provider::Chatgpt, &e).unwrap()),
            "0.000409"
        );
    }
    #[test]
    fn claude_ttl_dates_and_modifiers() {
        let mut e = event(
            "claude-haiku-4-5-20251001",
            Tokens {
                input: 100,
                read: 20,
                write: 10,
                write_1h: 10,
                output: 5,
                ..Tokens::default()
            },
            100,
        );
        assert_eq!(
            usd(Pricing::new().cost(Provider::Claude, &e).unwrap()),
            "0.000147"
        );
        e.us_geo = true;
        assert_eq!(
            usd(Pricing::new().cost(Provider::Claude, &e).unwrap()),
            "0.0001617"
        );
        e.tokens.write_1h = 0;
        assert!(Pricing::new().cost(Provider::Claude, &e).is_none());
        e.model = "unknown".into();
        assert!(Pricing::new().cost(Provider::Claude, &e).is_none());
    }
}
