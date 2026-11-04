export const ENTRY_REFRESH_INTERVAL = 180000;

export function canRefreshOnEntry(lastAttempt, now = Date.now()) {
  return !(Number.isFinite(lastAttempt) && lastAttempt > 0 && now - lastAttempt < ENTRY_REFRESH_INTERVAL);
}

export function relativeReset(reset, now = Date.now()) {
  if (!Number.isFinite(reset) || reset <= 0) return "";
  const remaining = reset - now;
  if (remaining <= 0) return "已到期";
  if (remaining >= 86400000) return `${Math.floor(remaining / 86400000)} 天后`;
  if (remaining >= 3600000) return `${Math.floor(remaining / 3600000)} 小时后`;
  if (remaining >= 60000) return `${Math.floor(remaining / 60000)} 分钟后`;
  return "不到 1 分钟后";
}

const validNumber = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER;
const validTime = (value) => value === null || validNumber(value) && value <= 8640000000000000;
const labels = { "5h": "5 小时", "7d": "7 天", "30d": "标准月额度" };
function validUsage(usage) {
  return usage === null || usage && typeof usage === "object" &&
    ["total_requests", "total_tokens", "total_cost", "success_calls", "failure_calls"].every((key) => validNumber(usage[key])) &&
    (usage.success_rate === null || validNumber(usage.success_rate) && usage.success_rate <= 1) &&
    validTime(usage.last_seen_ms) && ["ready", "empty"].includes(usage.sync_status) &&
    ["complete", "partial", "unmatched"].includes(usage.scope_match_status) && typeof usage.matched === "boolean";
}

// sessionStorage survives page reloads but not a closed tab. No token is stored;
// callers derive the key from the current management connection and authorization.
export function restoreRefreshSnapshot(raw, now = Date.now()) {
  try {
    if (!raw || raw.length > 2000000) return [];
    const snapshot = JSON.parse(raw);
    if (snapshot.version !== 1 || !Array.isArray(snapshot.entries) || snapshot.entries.length > 200) return [];
    return snapshot.entries.filter((entry) => {
      if (!Array.isArray(entry) || entry.length !== 2) return false;
      const [key, value] = entry;
      if (typeof key !== "string" || key.length > 1024 || !value || !validNumber(value.at) ||
          !validNumber(value.attemptAt) || value.attemptAt > now || canRefreshOnEntry(value.attemptAt, now)) return false;
      if (!Array.isArray(value.windows) || value.windows.length > 3) return false;
      if (![value.error, value.usageError].every((text) => typeof text === "string" && text.length <= 500)) return false;
      return value.windows.every((window) =>
        Object.hasOwn(labels, window.name) && window.label === labels[window.name] &&
        [window.start, window.end, window.reset, window.observedAt].every(validTime) &&
        validNumber(window.duration) && window.duration > 0 &&
        (window.percent === null || validNumber(window.percent) && window.percent <= 100) &&
        ["exact", "derived", "unknown"].includes(window.accuracy) &&
        typeof window.forecastEligible === "boolean" && typeof window.forecastReason === "string" &&
        ["上个窗口用量", "前一等长区间"].includes(window.previousLabel) &&
        (window.previousRange === null || window.previousRange && validTime(window.previousRange.from) && validTime(window.previousRange.to)) &&
        validUsage(window.current) && validUsage(window.previous)
      );
    }).map(([key, value]) => [key, { ...value, loading: false, usageLoading: false }]);
  } catch { return []; }
}

export function serializeRefreshSnapshot(cache, now = Date.now()) {
  return JSON.stringify({
    version: 1,
    entries: [...cache].filter(([, value]) => validNumber(value.attemptAt) && now - value.attemptAt < ENTRY_REFRESH_INTERVAL)
      .slice(-200).map(([key, value]) => [key, {
        at: value.at || 0, attemptAt: value.attemptAt,
        windows: value.windows || [], error: value.error || "", usageError: value.usageError || ""
      }])
  });
}
