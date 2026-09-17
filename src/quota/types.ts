export type Availability = "allowed" | "blocked" | "unknown" | "unavailable";

export type QuotaWindow = {
  id: string;
  name: string;
  usedPercent: number;
  remainingPercent: number;
  windowDurationMins: number | null;
  resetsAt: number | null;
};

export type QuotaSnapshot = {
  availability: Availability;
  windows: QuotaWindow[];
  planType: string | null;
  fetchedAt: number;
  stale: boolean;
};

