// Manager v1.14.10: accountWindowUsageRows / estimateWindowUsage contracts.
// https://github.com/seakee/CPA-Manager-Plus (MIT); see docs/integrations/cpa-manager-plus.
const DURATIONS = { fiveHour: 18000000, weekly: 604800000, monthly: 2592000000 };
const NAMES = { fiveHour: "5h", weekly: "7d", monthly: "30d" };
const LABELS = { fiveHour: "5 小时", weekly: "7 天", monthly: "标准月额度" };
const finite = (value) => typeof value === "number" && Number.isFinite(value);
const timestamp = (value) => {
  const parsed = typeof value === "number" ? value : typeof value === "string" && value ? Date.parse(value) : NaN;
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
};

export function normalizeQuota(data, account) {
  const quota = data?.data;
  if (!data?.ok || quota?.provider !== "factory-droid" || quota.auth_index !== account.index || quota.name !== account.name || !Array.isArray(quota.windows))
    throw new Error("额度响应与当前凭证不匹配");
  const observedAt = timestamp(quota.observed_at_ms);
  if (!observedAt || Math.abs(Date.now() - observedAt) > 300000) throw new Error("额度观测时间无效，请检查服务器时间");
  const windows = [];
  for (const raw of quota.windows) {
    const nominalDuration = DURATIONS[raw.window_key];
    if (raw.group !== "standard" || !nominalDuration || windows.some((item) => item.name === NAMES[raw.window_key])) continue;
    const end = timestamp(raw.window_end);
    const providedStart = timestamp(raw.window_start);
    const start = providedStart ?? (end ? end - nominalDuration : null);
    const duration = providedStart && end ? end - providedStart : nominalDuration;
    // A concrete reset identifies an anchored quota window. Without it we must
    // not turn a sliding lookback into an official billing interval.
    const usable = start && end && start > 0 && start < observedAt && observedAt < end &&
      duration > 0 && duration <= nominalDuration * 2 &&
      raw.reset_source === "window_end";
    const percent = finite(raw.remaining_fraction) ? Math.min(100, Math.max(0, raw.remaining_fraction * 100)) : null;
    windows.push({
      name: NAMES[raw.window_key], label: LABELS[raw.window_key], percent, reset: end,
      duration, start: usable ? start : null, end: usable ? end : null,
      accuracy: usable ? providedStart ? "exact" : "derived" : "unknown",
      observedAt, forecastEligible: quota.forecast_eligible === true, forecastReason: quota.forecast_reason || "",
      current: null, previous: null, previousRange: null, previousLabel: "前一等长区间"
    });
  }
  return windows.sort((a, b) => a.duration - b.duration);
}

export function buildTargets(account, windows, previousWindows = []) {
  const targets = [];
  for (const window of windows) {
    if (!window.start || !window.end) continue;
    const earlier = previousWindows.find((item) => item.name === window.name);
    // Only call it a previous window when its boundary was actually observed.
    const completed = earlier?.start && earlier.end <= window.start && earlier.end > window.start - window.duration
      ? { from: earlier.start, to: earlier.end } : null;
    const remembered = earlier?.start === window.start ? earlier.previousRange : null;
    window.previousRange = completed || remembered || { from: window.start - window.duration, to: window.start };
    window.previousLabel = completed || (remembered && earlier.previousLabel === "上个窗口用量") ? "上个窗口用量" : "前一等长区间";
    for (const [period, from, to] of [
      ["current", window.start, Math.min(window.observedAt, window.end)],
      [window.previousLabel === "上个窗口用量" ? "previous" : "previous_equal_range", window.previousRange.from, window.previousRange.to]
    ]) {
      if (from <= 0 || from >= to) continue;
      const id = `factory-standard-${window.name}`;
      targets.push({
        request_key: JSON.stringify([account.key, id, period, from, to]),
        row_key: account.key, window_key: id, provider_window_id: id, period,
        from_ms: from, to_ms: to, model_scope: { kind: "all", complete: true },
        auth_file_snapshot: account.name, auth_provider_snapshot: "factory-droid",
        auth_index: account.index, source: account.name,
        ...(account.account ? { account_snapshot: account.account } : {}),
        ...(account.label ? { auth_label_snapshot: account.label } : {})
      });
    }
  }
  return targets;
}

function validUsage(item, target) {
  return item.request_key === target.request_key && item.row_key === target.row_key &&
    item.provider_window_id === target.provider_window_id && item.period === target.period &&
    item.from_ms === target.from_ms && item.to_ms === target.to_ms &&
    ["ready", "empty"].includes(item.sync_status) && typeof item.matched === "boolean" &&
    ["total_requests", "total_tokens", "total_cost", "success_calls", "failure_calls"].every((key) => finite(item[key]) && item[key] >= 0) &&
    (item.last_seen_ms == null || (finite(item.last_seen_ms) && item.last_seen_ms >= target.from_ms && item.last_seen_ms < target.to_ms)) &&
    (item.success_rate === null || (finite(item.success_rate) && item.success_rate >= 0 && item.success_rate <= 1));
}

export function mergeUsage(windows, targets, response) {
  if (!Array.isArray(response?.items)) throw new Error("Manager 未返回窗口统计");
  for (const target of targets) {
    const matches = response.items.filter((item) => item.request_key === target.request_key);
    if (matches.length !== 1 || !validUsage(matches[0], target)) throw new Error("窗口统计响应不完整或与凭证不匹配");
  }
  return windows.map((window) => {
    const match = (period) => {
      const target = targets.find((item) => item.provider_window_id === `factory-standard-${window.name}` && (period === "current" ? item.period === period : item.period !== "current"));
      return target ? response.items.find((item) => item.request_key === target.request_key) : null;
    };
    return { ...window, current: match("current"), previous: match("previous") };
  });
}

export function actual(usage) {
  return usage?.matched === true && usage.sync_status === "ready" && usage.scope_match_status === "complete" &&
    ["total_requests", "total_tokens", "total_cost"].every((key) => finite(usage[key]) && usage[key] >= 0) ? usage : null;
}

export function forecast(window, { stale = false, usageError = "", now = Date.now() } = {}) {
  const current = actual(window.current);
  const used = window.percent === null ? null : 100 - window.percent;
  if (stale || usageError || !current || !window.forecastEligible || !window.start || !window.end ||
      now >= window.end || now - window.observedAt > 120000 || !finite(used) || used <= 0 || used >= 100 ||
      !finite(current.last_seen_ms) || current.last_seen_ms > window.observedAt ||
      !(current.total_requests > 0 || current.total_tokens > 0 || current.total_cost > 0)) return null;
  // Same quota-ratio estimate and rounding as Manager's estimateWindowUsage.
  // Keep Manager's priced cost; this module never calculates model prices.
  const multiplier = 100 / used;
  const result = {
    requests: Math.max(current.total_requests, Math.round(current.total_requests * multiplier)),
    tokens: Math.max(current.total_tokens, Math.round(current.total_tokens * multiplier)),
    cost: Math.max(current.total_cost, Math.round(current.total_cost * multiplier * 100) / 100)
  };
  return Object.values(result).every(finite) ? result : null;
}
