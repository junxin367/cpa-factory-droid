use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const SCALE: u128 = 1_000_000;
const MAX_PRICE: f64 = 1_000_000.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PriceRule {
    pub input_tokens_above: Option<u64>,
    pub service_tier: Option<String>,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Price {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub rules: Vec<PriceRule>,
}

pub fn service_tier(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "" | "auto" | "default" | "standard" => None,
        "fast" | "priority" => Some("priority".into()),
        "batches" | "batch" => Some("batch".into()),
        _ => Some(value),
    }
}

fn valid_rate(value: Option<f64>) -> bool {
    value.is_none_or(|rate| rate.is_finite() && (0.0..=MAX_PRICE).contains(&rate))
}

fn micro_rate(value: f64) -> u128 {
    (value * SCALE as f64).round() as u128
}

fn rates(
    input: Option<f64>,
    output: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
) -> Option<[u128; 4]> {
    let input = input?;
    let output = output?;
    Some([
        micro_rate(input),
        micro_rate(output),
        micro_rate(read.unwrap_or(input)),
        micro_rate(write.unwrap_or(input)),
    ])
}

impl PriceRule {
    fn rates(&self) -> Option<[u128; 4]> {
        rates(self.input, self.output, self.cache_read, self.cache_write)
    }

    fn validate(&self) -> Result<(), String> {
        if self.input_tokens_above.is_some_and(|value| value == 0)
            || (self.input_tokens_above.is_none() && self.service_tier.is_none())
        {
            return Err("计费规则需指定有效的输入 Token 门槛或服务等级".into());
        }
        if self
            .service_tier
            .as_deref()
            .is_some_and(|tier| !["priority", "flex", "batch", "ultrafast"].contains(&tier))
        {
            return Err("服务等级必须是 priority、flex、batch 或 ultrafast".into());
        }
        if self.input.is_none()
            || self.output.is_none()
            || ![self.input, self.output, self.cache_read, self.cache_write]
                .into_iter()
                .all(valid_rate)
        {
            return Err("计费规则的输入和输出价必填，价格须为非负有限数".into());
        }
        Ok(())
    }
}

impl Price {
    pub fn validate(&self) -> Result<(), String> {
        if ![self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .all(valid_rate)
        {
            return Err("模型价格必须为非负有限数".into());
        }
        if self.rules.len() > 64 {
            return Err("计费规则最多支持 64 条".into());
        }
        let mut conditions = BTreeSet::new();
        for rule in &self.rules {
            rule.validate()?;
            if !conditions.insert((rule.input_tokens_above, rule.service_tier.as_deref())) {
                return Err("同一输入 Token 门槛和服务等级不能重复设置".into());
            }
        }
        Ok(())
    }

    fn rates_for(&self, input: u64, tier: Option<&str>) -> Option<[u128; 4]> {
        let select = |tier| {
            self.rules
                .iter()
                .filter(|rule| {
                    rule.service_tier.as_deref() == tier
                        && rule
                            .input_tokens_above
                            .is_none_or(|threshold| input > threshold)
                })
                .max_by_key(|rule| rule.input_tokens_above.unwrap_or(0))
        };
        let context = select(None);
        if tier.is_none() {
            return context.map_or_else(
                || rates(self.input, self.output, self.cache_read, self.cache_write),
                PriceRule::rates,
            );
        }
        select(tier).and_then(PriceRule::rates)
    }

    pub fn cost(
        &self,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        tier: Option<&str>,
    ) -> Option<f64> {
        let total_input = input.checked_add(cache_read)?.checked_add(cache_write)?;
        let tier = tier.and_then(service_tier);
        let rates = self.rates_for(total_input, tier.as_deref())?;
        let sum = [input, output, cache_read, cache_write]
            .into_iter()
            .zip(rates)
            .map(|(count, rate)| u128::from(count) * rate)
            .sum::<u128>();
        let micros = sum.checked_add(SCALE - 1)? / SCALE;
        Some(micros as f64 / SCALE as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price() -> Price {
        Price {
            input: Some(10.0),
            output: Some(50.0),
            cache_read: None,
            cache_write: None,
            rules: vec![
                PriceRule {
                    input_tokens_above: Some(272_000),
                    input: Some(20.0),
                    output: Some(75.0),
                    cache_read: Some(2.0),
                    cache_write: Some(25.0),
                    ..PriceRule::default()
                },
                PriceRule {
                    service_tier: Some("priority".into()),
                    input: Some(20.0),
                    output: Some(100.0),
                    ..PriceRule::default()
                },
            ],
        }
    }

    #[test]
    fn cache_prices_default_to_input_and_each_request_rounds_up() {
        let flat = Price {
            input: Some(0.000001),
            output: Some(0.000001),
            ..Price::default()
        };
        assert_eq!(flat.cost(1, 1, 1, 1, None), Some(0.000001));
        let price = price();
        assert_eq!(price.cost(500, 1_000, 200, 300, None), Some(0.06));
    }

    #[test]
    fn context_and_service_rules_follow_reference_precedence() {
        let price = price();
        assert_eq!(price.cost(272_000, 1_000, 0, 0, None), Some(2.77));
        assert_eq!(price.cost(272_001, 1_000, 0, 0, None), Some(5.51502));
        assert_eq!(price.cost(1_000, 1_000, 0, 0, Some("fast")), Some(0.12));
        assert_eq!(price.cost(1_000, 1_000, 0, 0, Some("standard")), Some(0.06));
        assert_eq!(price.cost(1_000, 1_000, 0, 0, Some("batch")), None);
    }

    #[test]
    fn duplicate_or_incomplete_rules_are_rejected() {
        let mut duplicated = price();
        duplicated.rules.push(duplicated.rules[0].clone());
        assert!(duplicated.validate().is_err());
        let mut incomplete = price();
        incomplete.rules[0].input = None;
        assert!(incomplete.validate().is_err());
    }
}
