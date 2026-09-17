import { describe, expect, it } from "vitest";
import {
  formatResetCountdown,
  formatWindowLabel,
  normalizeQuotaResponse,
  selectTightestWindow,
} from "./normalize";

describe("normalizeQuotaResponse", () => {
  it("规范化旧版顶层 rateLimits 响应", () => {
    expect(
      normalizeQuotaResponse(
        {
          ordinaryUsageAllowed: true,
          planType: "plus",
          rateLimits: {
            primary: {
              usedPercent: 64,
              windowDurationMins: 300,
              resetsAt: 1_789_588_800,
            },
            secondary: {
              usedPercent: 18,
              windowDurationMins: 10_080,
              resetsAt: 1_790_190_000,
            },
          },
        },
        1_789_585_200,
      ),
    ).toEqual({
      availability: "allowed",
      planType: "plus",
      fetchedAt: 1_789_585_200,
      stale: false,
      windows: [
        {
          id: "primary",
          name: "5H",
          usedPercent: 64,
          remainingPercent: 36,
          windowDurationMins: 300,
          resetsAt: 1_789_588_800,
        },
        {
          id: "secondary",
          name: "Weekly",
          usedPercent: 18,
          remainingPercent: 82,
          windowDurationMins: 10_080,
          resetsAt: 1_790_190_000,
        },
      ],
    });
  });

  it("优先使用 rateLimitsByLimitId 中的 codex 桶", () => {
    const snapshot = normalizeQuotaResponse({
      rateLimits: { primary: { usedPercent: 90, windowDurationMins: 60 } },
      rateLimitsByLimitId: {
        other: { primary: { usedPercent: 5, windowDurationMins: 60 } },
        codex: { primary: { usedPercent: 25, windowDurationMins: 60 } },
      },
    });

    expect(snapshot.windows).toEqual([
      expect.objectContaining({ id: "primary", usedPercent: 25, remainingPercent: 75 }),
    ]);
  });

  it("保留仅有的周额度窗口", () => {
    const snapshot = normalizeQuotaResponse({
      rateLimits: {
        secondary: { usedPercent: 10, windowDurationMins: 10_080, resetsAt: 500 },
      },
    });

    expect(snapshot.windows).toEqual([
      expect.objectContaining({ id: "secondary", name: "Weekly", remainingPercent: 90 }),
    ]);
  });

  it("忽略空窗口、未知套餐及新增协议字段", () => {
    expect(() =>
      normalizeQuotaResponse({
        planType: { unexpected: true },
        rateLimits: { primary: null, secondary: null },
        futureProtocolField: ["ignored"],
      }),
    ).not.toThrow();

    expect(
      normalizeQuotaResponse({
        planType: { unexpected: true },
        rateLimits: { primary: null, secondary: null },
        futureProtocolField: ["ignored"],
      }),
    ).toMatchObject({ planType: null, availability: "unknown", windows: [] });
  });

  it("钳制百分比并映射 blocked 与 unknown 可用性", () => {
    const blocked = normalizeQuotaResponse({
      ordinaryUsageAllowed: false,
      rateLimits: {
        primary: { usedPercent: 140 },
        secondary: { usedPercent: -10 },
      },
    });
    const unknown = normalizeQuotaResponse({
      ordinaryUsageAllowed: null,
      rateLimits: { primary: { usedPercent: 40 } },
    });

    expect(blocked.availability).toBe("blocked");
    expect(blocked.windows).toEqual([
      expect.objectContaining({ usedPercent: 100, remainingPercent: 0 }),
      expect.objectContaining({ usedPercent: 0, remainingPercent: 100 }),
    ]);
    expect(unknown.availability).toBe("unknown");
  });

  it("将显式 null 的可用性保留为 unknown", () => {
    const snapshot = normalizeQuotaResponse({
      ordinaryUsageAllowed: null,
      rateLimitsByLimitId: {
        codex: {
          ordinaryUsageAllowed: true,
          primary: { usedPercent: 40 },
        },
      },
    });

    expect(snapshot.availability).toBe("unknown");
  });

  it("基于动态时长标记窗口，并在倒计时边界保持非负", () => {
    expect(formatWindowLabel(null)).toBe("Unknown");
    expect(formatWindowLabel(10_080)).toBe("Weekly");
    expect(formatWindowLabel(2_880)).toBe("2D");
    expect(formatWindowLabel(300)).toBe("5H");
    expect(formatWindowLabel(90)).toBe("90M");
    expect(formatResetCountdown(null, 100)).toBe("重置时间未知");
    expect(formatResetCountdown(99, 100)).toBe("0M");
    expect(formatResetCountdown(3_760, 100)).toBe("1H 1M");
  });

  it("选择剩余额度最少的有效窗口", () => {
    expect(
      selectTightestWindow([
        { id: "invalid", name: "Unknown", usedPercent: 110, remainingPercent: -10, windowDurationMins: null, resetsAt: null },
        { id: "primary", name: "5H", usedPercent: 10, remainingPercent: 90, windowDurationMins: 300, resetsAt: null },
        { id: "secondary", name: "Weekly", usedPercent: 80, remainingPercent: 20, windowDurationMins: 10_080, resetsAt: null },
      ]),
    ).toMatchObject({ id: "secondary" });
    expect(selectTightestWindow([])).toBeNull();
  });
});
