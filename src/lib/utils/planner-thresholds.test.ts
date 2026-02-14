import { describe, expect, it } from "vitest";

import {
  plannerThresholdsFromCeilingFraction,
  plannerThresholdsFromCeilingPercent,
} from "./planner-thresholds";

describe("planner threshold helpers", () => {
  it("derives soft/medium/hard at 50/80/100 of budget ceiling", () => {
    const thresholds = plannerThresholdsFromCeilingFraction(0.8);
    expect(thresholds.soft).toBeCloseTo(0.4, 6);
    expect(thresholds.medium).toBeCloseTo(0.64, 6);
    expect(thresholds.hard).toBeCloseTo(0.8, 6);
  });

  it("clamps budget ceiling to supported range before deriving thresholds", () => {
    const low = plannerThresholdsFromCeilingFraction(0.1);
    expect(low.ceiling).toBeCloseTo(0.4, 6);
    expect(low.soft).toBeCloseTo(0.2, 6);

    const high = plannerThresholdsFromCeilingFraction(2.0);
    expect(high.ceiling).toBeCloseTo(1.0, 6);
    expect(high.hard).toBeCloseTo(1.0, 6);
  });

  it("returns rounded percentages for settings UI", () => {
    const thresholds = plannerThresholdsFromCeilingPercent(80);
    expect(thresholds.soft).toBe(40);
    expect(thresholds.medium).toBe(64);
    expect(thresholds.hard).toBe(80);
  });
});
