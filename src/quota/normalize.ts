import type { Availability, QuotaSnapshot, QuotaWindow } from "./types";

type RecordValue = Record<string, unknown>;

function isRecord(value: unknown): value is RecordValue {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asFiniteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function clampPercent(value: unknown): number {
  const number = asFiniteNumber(value) ?? 0;
  return Math.min(100, Math.max(0, number));
}

function optionalNumber(value: unknown): number | null {
  return asFiniteNumber(value);
}

function availabilityFor(value: unknown): Availability {
  if (value === true) return "allowed";
  if (value === false) return "blocked";
  return "unknown";
}

function rootValueOrBucketValue(root: RecordValue, bucket: RecordValue | null, key: string): unknown {
  return Object.prototype.hasOwnProperty.call(root, key) ? root[key] : bucket?.[key];
}

function quotaBucket(response: RecordValue): RecordValue | null {
  const byLimitId = response.rateLimitsByLimitId;
  if (isRecord(byLimitId) && isRecord(byLimitId.codex)) return byLimitId.codex;
  return isRecord(response.rateLimits) ? response.rateLimits : null;
}

function normalizeWindow(id: "primary" | "secondary", value: unknown): QuotaWindow | null {
  if (!isRecord(value)) return null;

  const usedPercent = clampPercent(value.usedPercent);
  const windowDurationMins = optionalNumber(value.windowDurationMins);

  return {
    id,
    name: formatWindowLabel(windowDurationMins),
    usedPercent,
    remainingPercent: 100 - usedPercent,
    windowDurationMins,
    resetsAt: optionalNumber(value.resetsAt),
  };
}

/** Converts a permissive App Server response into the UI's stable quota model. */
export function normalizeQuotaResponse(
  response: unknown,
  fetchedAt = Math.floor(Date.now() / 1000),
): QuotaSnapshot {
  const root = isRecord(response) ? response : {};
  const bucket = quotaBucket(root);
  const windows = bucket
    ? (["primary", "secondary"] as const)
        .map((id) => normalizeWindow(id, bucket[id]))
        .filter((window): window is QuotaWindow => window !== null)
    : [];
  const availabilityValue = rootValueOrBucketValue(root, bucket, "ordinaryUsageAllowed");
  const planTypeValue = rootValueOrBucketValue(root, bucket, "planType");

  return {
    availability: availabilityFor(availabilityValue),
    windows,
    planType: typeof planTypeValue === "string" ? planTypeValue : null,
    fetchedAt: asFiniteNumber(fetchedAt) ?? Math.floor(Date.now() / 1000),
    stale: false,
  };
}

/** Produces the compact label shown for a quota window duration. */
export function formatWindowLabel(windowDurationMins: number | null): string {
  if (windowDurationMins === null) return "Unknown";
  if (windowDurationMins === 10_080) return "Weekly";
  if (Number.isInteger(windowDurationMins) && windowDurationMins > 0 && windowDurationMins % 1_440 === 0) {
    return `${windowDurationMins / 1_440}D`;
  }
  if (Number.isInteger(windowDurationMins) && windowDurationMins > 0 && windowDurationMins % 60 === 0) {
    return `${windowDurationMins / 60}H`;
  }
  return `${windowDurationMins}m`;
}

/** Returns the window with the least remaining quota, or null when none are usable. */
export function selectTightestWindow(windows: readonly QuotaWindow[]): QuotaWindow | null {
  return windows.reduce<QuotaWindow | null>((tightest, window) => {
    if (!Number.isFinite(window.remainingPercent) || window.remainingPercent < 0 || window.remainingPercent > 100) {
      return tightest;
    }
    if (tightest === null || window.remainingPercent < tightest.remainingPercent) return window;
    return tightest;
  }, null);
}

/** Formats a non-negative countdown from Unix-second timestamps. */
export function formatResetCountdown(resetsAt: number | null, now = Math.floor(Date.now() / 1000)): string {
  if (resetsAt === null || !Number.isFinite(resetsAt)) return "重置时间未知";

  const nowSeconds = Number.isFinite(now) ? now : Math.floor(Date.now() / 1000);
  const remainingMinutes = Math.max(0, Math.floor((resetsAt - nowSeconds) / 60));
  const days = Math.floor(remainingMinutes / 1_440);
  const hours = Math.floor((remainingMinutes % 1_440) / 60);
  const minutes = remainingMinutes % 60;

  if (days > 0) return `${days}D ${hours}H`;
  if (hours > 0) return `${hours}H ${minutes}M`;
  return `${minutes}M`;
}
