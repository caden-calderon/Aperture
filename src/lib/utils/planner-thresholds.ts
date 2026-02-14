const MIN_BUDGET_CEILING = 0.4;
const MAX_BUDGET_CEILING = 1.0;

export const PLANNER_THRESHOLD_MULTIPLIERS = {
  soft: 0.5,
  medium: 0.8,
  hard: 1.0,
} as const;

export interface PlannerThresholdFractions {
  soft: number;
  medium: number;
  hard: number;
  ceiling: number;
}

export interface PlannerThresholdPercents {
  soft: number;
  medium: number;
  hard: number;
  ceiling: number;
}

function clampCeilingFraction(ceiling: number): number {
  return Math.max(MIN_BUDGET_CEILING, Math.min(MAX_BUDGET_CEILING, ceiling));
}

export function plannerThresholdsFromCeilingFraction(ceiling: number): PlannerThresholdFractions {
  const clamped = clampCeilingFraction(ceiling);
  return {
    soft: clamped * PLANNER_THRESHOLD_MULTIPLIERS.soft,
    medium: clamped * PLANNER_THRESHOLD_MULTIPLIERS.medium,
    hard: clamped * PLANNER_THRESHOLD_MULTIPLIERS.hard,
    ceiling: clamped,
  };
}

export function plannerThresholdsFromCeilingPercent(ceilingPercent: number): PlannerThresholdPercents {
  const fractions = plannerThresholdsFromCeilingFraction(ceilingPercent / 100);
  return {
    soft: Math.round(fractions.soft * 100),
    medium: Math.round(fractions.medium * 100),
    hard: Math.round(fractions.hard * 100),
    ceiling: Math.round(fractions.ceiling * 100),
  };
}
