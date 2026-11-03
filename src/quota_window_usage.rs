//! Pure local-ledger accounting for an explicit Factory quota window.
//!
//! Callers supply trusted, normalized Factory boundaries. This module never
//! substitutes the global reporting window or guesses a duration from a label.
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Persisted samples with earlier semantics must not be mixed with this version.
pub const SAMPLE_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default)]
pub struct RecordTokens {
    /// Unified input already includes cache reads and writes.
    pub input: u64,
    /// Output already includes reasoning tokens.
    pub output: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PeriodRecord<'a> {
    pub account_id: &'a str,
    /// Only explicit `standard` and `core` classifications are accepted.
    pub pool: &'a str,
    pub at: &'a str,
    pub success: bool,
    pub kind: &'a str,
    pub tokens: Option<RecordTokens>,
    /// Price saved with this request, never recalculated using today's settings.
    pub estimated_cost_usd: Option<f64>,
}

/// Evidence of continuous recording and continuous retention, independent of the
/// first request found in the ledger. Missing evidence means unknown coverage.
#[derive(Clone, Debug, Default)]
pub struct CoverageEvidence {
    pub observed_since: Option<String>,
    pub retained_since: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedWindow {
    pub status: &'static str,
    pub window_start: Option<String>,
    pub window_end: Option<String>,
    pub boundary_source: Option<&'static str>,
    pub reason: Option<&'static str>,
}

fn time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn valid_pool(pool: &str) -> bool {
    matches!(pool, "standard" | "core")
}

/// `window_start`/`window_end` must come from a trusted Factory parser. A reset
/// timestamp alone is insufficient. Derivation additionally requires a nonempty
/// machine `window_key`, positive `duration_seconds`, and
/// `duration_source = "factory_window_key"` supplied by that parser after
/// confirming the key's semantics. A human display name is never inspected.
pub fn resolve_window_bounds(window: &Value, now: &str) -> ResolvedWindow {
    let mut result = ResolvedWindow {
        status: "unknown",
        window_start: None,
        window_end: None,
        boundary_source: None,
        reason: Some("unknown_window_boundary"),
    };
    let Some(now) = time(now) else {
        result.reason = Some("invalid_current_time");
        return result;
    };
    let end_raw = window["window_end"]
        .as_str()
        .or_else(|| window["reset_at"].as_str());
    let Some(end) = end_raw.and_then(time) else {
        return result;
    };
    result.window_end = Some(timestamp(end));
    let start = if let Some(raw) = window["window_start"].as_str() {
        match time(raw) {
            Some(start) => {
                result.boundary_source = Some("official_window");
                Some(start)
            }
            None => {
                result.reason = Some("invalid_window_start");
                return result;
            }
        }
    } else if window["duration_source"] == "factory_window_key"
        && window["window_key"]
            .as_str()
            .is_some_and(|key| !key.is_empty())
    {
        window["duration_seconds"]
            .as_u64()
            .and_then(|n| i64::try_from(n).ok())
            .filter(|n| *n > 0)
            .and_then(Duration::try_seconds)
            .and_then(|duration| end.checked_sub_signed(duration))
            .inspect(|_| result.boundary_source = Some("factory_window_key_duration"))
    } else {
        None
    };
    let Some(start) = start else {
        return result;
    };
    result.window_start = Some(timestamp(start));
    if start >= end {
        result.reason = Some("invalid_window_interval");
    } else if now < start {
        result.reason = Some("future_window");
    } else if now >= end {
        result.reason = Some("expired_window");
    } else {
        result.status = "available";
        result.reason = None;
    }
    result
}

#[derive(Clone, Debug, Default)]
struct Totals {
    requests: u64,
    failed: u64,
    measured: u64,
    input: u64,
    output: u64,
    cached: u64,
    cache_write: u64,
    reasoning: u64,
    known_cost: f64,
    unpriced: u64,
    missing_usage: u64,
    overflow: bool,
}

fn add_u64(target: &mut u64, value: u64, overflow: &mut bool) {
    if let Some(sum) = target.checked_add(value) {
        *target = sum;
    } else {
        *target = u64::MAX;
        *overflow = true;
    }
}

fn known_price(value: Option<f64>) -> Option<f64> {
    value.filter(|cost| cost.is_finite() && *cost >= 0.0)
}

impl Totals {
    fn add(&mut self, record: &PeriodRecord<'_>) {
        if !record.success {
            add_u64(&mut self.failed, 1, &mut self.overflow);
            return;
        }
        add_u64(&mut self.requests, 1, &mut self.overflow);
        if let Some(tokens) = record.tokens {
            add_u64(&mut self.measured, 1, &mut self.overflow);
            add_u64(&mut self.input, tokens.input, &mut self.overflow);
            add_u64(&mut self.output, tokens.output, &mut self.overflow);
            add_u64(&mut self.cached, tokens.cached, &mut self.overflow);
            add_u64(
                &mut self.cache_write,
                tokens.cache_write,
                &mut self.overflow,
            );
            add_u64(&mut self.reasoning, tokens.reasoning, &mut self.overflow);
        } else {
            add_u64(&mut self.missing_usage, 1, &mut self.overflow);
        }
        if let Some(cost) = known_price(record.estimated_cost_usd) {
            self.known_cost += cost;
            self.overflow |= !self.known_cost.is_finite();
        } else {
            add_u64(&mut self.unpriced, 1, &mut self.overflow);
        }
    }

    fn json(&self) -> Value {
        let total = self.input.checked_add(self.output);
        let numeric_complete = !self.overflow && total.is_some();
        let cost_complete = self.unpriced == 0 && self.missing_usage == 0 && !self.overflow;
        json!({
            "request_count":self.requests,
            "successful_requests":self.requests,
            "failed_requests":self.failed,
            "total_requests":self.requests.checked_add(self.failed),
            "measured_count":self.measured,
            "input_tokens":if numeric_complete {Some(self.input)} else {None},
            "output_tokens":if numeric_complete {Some(self.output)} else {None},
            "total_tokens":if numeric_complete {total} else {None},
            "total_mtokens":if numeric_complete {total.map(|n|n as f64/1_000_000.0)} else {None},
            "cached_input_tokens":self.cached,
            "cache_write_tokens":self.cache_write,
            "reasoning_tokens":self.reasoning,
            "known_cost_usd":if self.known_cost.is_finite(){Some(self.known_cost)}else{None},
            "estimated_cost_usd":if cost_complete {Some(self.known_cost)}else{None},
            "cost_complete":cost_complete,
            "usage_complete":self.missing_usage==0 && numeric_complete,
            "unpriced_requests":self.unpriced,
            "missing_usage_requests":self.missing_usage,
            "arithmetic_overflow":!numeric_complete
        })
    }
}

fn coverage_json(
    evidence: &CoverageEvidence,
    bounds: &ResolvedWindow,
    observed_from: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Value {
    let observed_since = evidence.observed_since.as_deref().and_then(time);
    let retained_since = evidence.retained_since.as_deref().and_then(time);
    let start = bounds.window_start.as_deref().and_then(time);
    let coverage_start = observed_since
        .zip(retained_since)
        .filter(|(observed, retained)| *observed <= now && *retained <= now)
        .map(|(observed, retained)| observed.max(retained));
    let status = match (start, coverage_start) {
        (Some(start), Some(coverage)) if bounds.status == "available" => {
            if coverage <= start {
                "complete"
            } else {
                "partial"
            }
        }
        _ => "unknown",
    };
    json!({
        "coverage":status,
        "coverage_start":coverage_start.map(timestamp),
        "coverage_end":if bounds.status=="available"{Some(timestamp(now))}else{None},
        "observed_from":observed_from.map(timestamp),
        "observation_started_at":observed_since.map(timestamp),
        "retained_since":retained_since.map(timestamp),
        "coverage_scope":"plugin_observed",
        "coverage_message":"仅说明本插件记录的连续覆盖情况，不代表 Factory 全部客户端用量"
    })
}

/// Aggregate one account and one explicit pool over `[window_start, window_end)`.
/// Validation is included in main totals because it consumes quota, and also
/// returned separately. Failed requests never contribute tokens or costs.
pub fn aggregate_window_usage(
    records: &[PeriodRecord<'_>],
    account_id: &str,
    window: &Value,
    coverage: &CoverageEvidence,
    now: &str,
) -> Value {
    let bounds = resolve_window_bounds(window, now);
    let pool = window["group"].as_str().unwrap_or("");
    let mut row = json!({
        "status":"unknown",
        "account_id":account_id,
        "pool":pool,
        "window_start":bounds.window_start,
        "window_end":bounds.window_end,
        "boundary_source":bounds.boundary_source,
        "reason":bounds.reason,
        "scope":"plugin_observed",
        "unit":"raw_input_plus_output_tokens",
        "includes_validation":true,
        "request_count":null,
        "successful_requests":null,
        "failed_requests":null,
        "total_requests":null,
        "measured_count":null,
        "input_tokens":null,
        "output_tokens":null,
        "total_tokens":null,
        "total_mtokens":null,
        "cached_input_tokens":null,
        "cache_write_tokens":null,
        "reasoning_tokens":null,
        "known_cost_usd":null,
        "estimated_cost_usd":null,
        "cost_complete":false,
        "usage_complete":false,
        "unpriced_requests":null,
        "missing_usage_requests":null,
        "unclassified_requests":null,
        "unclassified_successful_requests":null,
        "unclassified_pool_usage_included":false,
        "invalid_timestamp_requests":null,
        "coverage":"unknown",
        "coverage_start":null,
        "coverage_end":null,
        "observed_from":null,
        "observation_started_at":null,
        "retained_since":null,
        "coverage_scope":"plugin_observed",
        "validation":null
    });
    if !valid_pool(pool) {
        row["reason"] = json!("unknown_pool");
        return row;
    }
    let (Some(start), Some(end), Some(now)) = (
        bounds.window_start.as_deref().and_then(time),
        bounds.window_end.as_deref().and_then(time),
        time(now),
    ) else {
        return row;
    };
    if bounds.status != "available" {
        return row;
    }
    let mut totals = Totals::default();
    let mut validation = Totals::default();
    let mut inference = Totals::default();
    let mut first = None;
    let mut unclassified_requests = 0u64;
    let mut unclassified_successful_requests = 0u64;
    let mut invalid_timestamp_requests = 0u64;
    for record in records.iter().filter(|r| r.account_id == account_id) {
        let Some(at) = time(record.at) else {
            if record.pool == pool || !valid_pool(record.pool) {
                invalid_timestamp_requests = invalid_timestamp_requests.saturating_add(1);
            }
            continue;
        };
        if at < start || at >= end {
            continue;
        }
        if !valid_pool(record.pool) {
            unclassified_requests = unclassified_requests.saturating_add(1);
            if record.success {
                unclassified_successful_requests =
                    unclassified_successful_requests.saturating_add(1);
            }
            continue;
        }
        if record.pool != pool {
            continue;
        }
        first = Some(first.map_or(at, |existing: DateTime<Utc>| existing.min(at)));
        totals.add(record);
        if record.kind == "validation" {
            validation.add(record);
        } else {
            inference.add(record);
        }
    }
    row.as_object_mut()
        .unwrap()
        .extend(totals.json().as_object().unwrap().clone());
    row.as_object_mut().unwrap().extend(
        coverage_json(coverage, &bounds, first, now)
            .as_object()
            .unwrap()
            .clone(),
    );
    row["status"] = json!("available");
    row["unclassified_requests"] = json!(unclassified_requests);
    row["unclassified_successful_requests"] = json!(unclassified_successful_requests);
    row["invalid_timestamp_requests"] = json!(invalid_timestamp_requests);
    row["validation"] = validation.json();
    row["inference"] = inference.json();
    row["validation_requests"] = json!(validation.requests);
    row["failed_validation_requests"] = json!(validation.failed);
    row["validation_total_tokens"] = row["validation"]["total_tokens"].clone();
    row["validation_total_mtokens"] = row["validation"]["total_mtokens"].clone();
    row["validation_known_cost_usd"] = row["validation"]["known_cost_usd"].clone();
    row["validation_estimated_cost_usd"] = row["validation"]["estimated_cost_usd"].clone();
    if invalid_timestamp_requests > 0 {
        row["usage_complete"] = json!(false);
        row["cost_complete"] = json!(false);
        row["estimated_cost_usd"] = Value::Null;
        row["coverage"] = json!("partial");
    }
    row["message"] = json!(
        "仅汇总本插件在官方周期内记录的原始输入加输出；缓存、reasoning 不重复相加；未归类记录不计入任何额度池"
    );
    row
}

/// Monotonic counters used by v2 quota samples. Missing prices before the
/// baseline do not poison a later fully priced sample interval.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CumulativeUsage {
    pub tokens: u64,
    pub known_cost_usd: f64,
    pub requests: u64,
    pub measured: u64,
    pub failed_requests: u64,
    pub unpriced_requests: u64,
    pub missing_usage_requests: u64,
    /// Successful records with unknown pool, never included in `tokens` or USD.
    pub unclassified_requests: u64,
    pub invalid_timestamp_requests: u64,
    pub arithmetic_overflow: bool,
}

/// Counters include successful inference and validation through `at` (inclusive).
/// Callers must preserve ledger retention/epoch consistency between samples.
pub fn cumulative_pool_usage(
    records: &[PeriodRecord<'_>],
    account_id: &str,
    pool: &str,
    at: &str,
) -> Option<CumulativeUsage> {
    if !valid_pool(pool) {
        return None;
    }
    let at = time(at)?;
    let mut totals = Totals::default();
    let mut unclassified_requests = 0u64;
    let mut invalid_timestamp_requests = 0u64;
    for record in records.iter().filter(|r| r.account_id == account_id) {
        if record.pool != pool && valid_pool(record.pool) {
            continue;
        }
        let Some(record_at) = time(record.at) else {
            invalid_timestamp_requests = invalid_timestamp_requests.saturating_add(1);
            continue;
        };
        if record_at > at {
            continue;
        }
        if !valid_pool(record.pool) {
            if record.success {
                unclassified_requests = unclassified_requests.saturating_add(1);
            }
            continue;
        }
        totals.add(record);
    }
    let tokens = totals.input.checked_add(totals.output);
    Some(CumulativeUsage {
        tokens: tokens.unwrap_or(u64::MAX),
        known_cost_usd: totals.known_cost,
        requests: totals.requests,
        measured: totals.measured,
        failed_requests: totals.failed,
        unpriced_requests: totals.unpriced,
        missing_usage_requests: totals.missing_usage,
        unclassified_requests,
        invalid_timestamp_requests,
        arithmetic_overflow: totals.overflow || tokens.is_none(),
    })
}

/// A point is scoped to an account, explicit pool, machine window identity and
/// reset boundary. `ledger_epoch` must change when retention or migration can
/// remove/rewrite cumulative entries; it is not an account credential revision.
#[derive(Clone, Debug)]
pub struct QuotaPoint {
    pub version: u32,
    pub ledger_epoch: String,
    pub account_id: String,
    pub pool: String,
    pub window_key: String,
    pub window_start: String,
    pub window_end: String,
    pub at: String,
    pub used_percent: f64,
    pub totals: CumulativeUsage,
}

fn sample_delta(base: &CumulativeUsage, current: &CumulativeUsage) -> Option<CumulativeUsage> {
    if base.arithmetic_overflow
        || current.arithmetic_overflow
        || !base.known_cost_usd.is_finite()
        || !current.known_cost_usd.is_finite()
        || base.known_cost_usd < 0.0
        || current.known_cost_usd < base.known_cost_usd
    {
        return None;
    }
    Some(CumulativeUsage {
        tokens: current.tokens.checked_sub(base.tokens)?,
        known_cost_usd: current.known_cost_usd - base.known_cost_usd,
        requests: current.requests.checked_sub(base.requests)?,
        measured: current.measured.checked_sub(base.measured)?,
        failed_requests: current.failed_requests.checked_sub(base.failed_requests)?,
        unpriced_requests: current
            .unpriced_requests
            .checked_sub(base.unpriced_requests)?,
        missing_usage_requests: current
            .missing_usage_requests
            .checked_sub(base.missing_usage_requests)?,
        unclassified_requests: current
            .unclassified_requests
            .checked_sub(base.unclassified_requests)?,
        invalid_timestamp_requests: current
            .invalid_timestamp_requests
            .checked_sub(base.invalid_timestamp_requests)?,
        arithmetic_overflow: false,
    })
}

/// Estimate quota capacity only from an eligible v2 pair. Missing pricing blocks
/// USD only; missing usage or unknown pool records in the interval block token
/// prediction. A missing official remaining fraction stays null.
pub fn estimate_sample_pair(
    base: &QuotaPoint,
    current: &QuotaPoint,
    remaining_fraction: Option<f64>,
    now: &str,
) -> Value {
    let mut result = json!({
        "status":"unknown",
        "reason":"incompatible_samples",
        "estimated":true,
        "estimated_total_tokens":null,
        "estimated_remaining_tokens":null,
        "estimated_total_mtokens":null,
        "estimated_remaining_mtokens":null,
        "estimated_total_cost_usd":null,
        "estimated_remaining_cost_usd":null,
        "sample_cost_usd":null,
        "cost_complete":false,
        "message":"需要同账号、同额度池、同官方周期的两次有效快照，额度变化至少 0.5 个百分点且至少 100 原始 token"
    });
    if base.version != SAMPLE_VERSION || current.version != SAMPLE_VERSION {
        result["reason"] = json!("unsupported_sample_version");
        return result;
    }
    if base.ledger_epoch.is_empty()
        || base.ledger_epoch != current.ledger_epoch
        || base.account_id.is_empty()
        || base.account_id != current.account_id
        || !valid_pool(&base.pool)
        || base.pool != current.pool
        || base.window_key.is_empty()
        || base.window_key != current.window_key
    {
        return result;
    }
    let (
        Some(base_at),
        Some(current_at),
        Some(base_start),
        Some(current_start),
        Some(base_end),
        Some(current_end),
        Some(now),
    ) = (
        time(&base.at),
        time(&current.at),
        time(&base.window_start),
        time(&current.window_start),
        time(&base.window_end),
        time(&current.window_end),
        time(now),
    )
    else {
        result["reason"] = json!("invalid_sample_time");
        return result;
    };
    if base_start != current_start
        || base_end != current_end
        || base_start >= base_end
        || base_at < base_start
        || base_at >= current_at
        || current_at >= base_end
        || current_at > now
        || now >= base_end
    {
        result["reason"] = json!("incompatible_window_or_time");
        return result;
    }
    if !base.used_percent.is_finite()
        || !current.used_percent.is_finite()
        || !(0.0..=100.0).contains(&base.used_percent)
        || !(0.0..=100.0).contains(&current.used_percent)
    {
        result["reason"] = json!("invalid_used_percent");
        return result;
    }
    let used_delta = current.used_percent - base.used_percent;
    if used_delta < 0.5 {
        result["reason"] = json!("insufficient_quota_change");
        return result;
    }
    let Some(delta) = sample_delta(&base.totals, &current.totals) else {
        result["reason"] = json!("cumulative_counters_changed");
        return result;
    };
    result["baseline_at"] = json!(base.at);
    result["sampled_at"] = json!(current.at);
    result["delta_used_percent"] = json!(used_delta);
    result["sample_tokens"] = json!(delta.tokens);
    result["sample_requests"] = json!(delta.requests);
    result["sample_measured_count"] = json!(delta.measured);
    result["sample_unpriced_requests"] = json!(delta.unpriced_requests);
    result["sample_missing_usage_requests"] = json!(delta.missing_usage_requests);
    result["sample_unclassified_requests"] = json!(delta.unclassified_requests);
    if delta.invalid_timestamp_requests > 0 {
        result["reason"] = json!("invalid_timestamp_in_sample");
        return result;
    }
    if delta.unclassified_requests > 0 {
        result["reason"] = json!("unclassified_pool_in_sample");
        return result;
    }
    if delta.requests == 0 || delta.measured != delta.requests || delta.missing_usage_requests > 0 {
        result["reason"] = json!("missing_usage_in_sample");
        return result;
    }
    if delta.tokens < 100 {
        result["reason"] = json!("insufficient_sample_tokens");
        return result;
    }
    let fraction = used_delta / 100.0;
    let total = delta.tokens as f64 / fraction;
    let remaining = remaining_fraction.filter(|n| n.is_finite() && (0.0..=1.0).contains(n));
    let total_cost = (delta.unpriced_requests == 0)
        .then_some(delta.known_cost_usd / fraction)
        .filter(|cost| cost.is_finite());
    result["status"] = json!("estimated");
    result["reason"] = Value::Null;
    result["remaining_fraction"] = json!(remaining);
    result["estimated_total_tokens"] = json!(total);
    result["estimated_remaining_tokens"] = json!(remaining.map(|n| total * n));
    result["estimated_total_mtokens"] = json!(total / 1_000_000.0);
    result["estimated_remaining_mtokens"] = json!(remaining.map(|n| total * n / 1_000_000.0));
    result["sample_cost_usd"] = json!(if total_cost.is_some() {
        Some(delta.known_cost_usd)
    } else {
        None
    });
    result["cost_complete"] = json!(total_cost.is_some());
    result["estimated_total_cost_usd"] = json!(total_cost);
    result["estimated_remaining_cost_usd"] =
        json!(total_cost.zip(remaining).map(|(cost, n)| cost * n));
    result["message"] = json!(
        "按本插件样本和官方额度百分比变化推算；其他客户端用量、模型组合变化和百分比精度会影响结果；不是官方 token 或美元余额"
    );
    result
}
