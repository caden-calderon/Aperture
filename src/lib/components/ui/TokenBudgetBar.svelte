<script lang="ts">
  import type { TokenBudget } from "$lib/types";
  import { plannerThresholdsFromCeilingFraction } from "$lib/utils/planner-thresholds";

  interface ProviderUsageSnapshot {
    requestId: string;
    inputTokens: number | null;
    outputTokens: number | null;
    totalTokens: number | null;
    observedAt: number;
  }

  interface Props {
    budget: TokenBudget;
    limit?: number;
    providerUsage?: ProviderUsageSnapshot | null;
    budgetCeiling?: number;
  }

  let { budget, limit = 200000, providerUsage = null, budgetCeiling = 0.80 }: Props = $props();
  let plannerThresholds = $derived(plannerThresholdsFromCeilingFraction(budgetCeiling));

  let percentUsed = $derived(Math.min((budget.used / limit) * 100, 100));
  let freeTokens = $derived(limit - budget.used);
  let providerTotal = $derived.by(() => {
    if (!providerUsage) return null;
    if (providerUsage.totalTokens !== null) return providerUsage.totalTokens;
    if (providerUsage.inputTokens !== null || providerUsage.outputTokens !== null) {
      return (providerUsage.inputTokens ?? 0) + (providerUsage.outputTokens ?? 0);
    }
    return null;
  });
  let providerTitle = $derived.by(() => {
    if (!providerUsage) return "";
    const parts: string[] = [];
    if (providerUsage.totalTokens !== null) {
      parts.push(`total ${formatNumber(providerUsage.totalTokens)}`);
    }
    if (providerUsage.inputTokens !== null) {
      parts.push(`input ${formatNumber(providerUsage.inputTokens)}`);
    }
    if (providerUsage.outputTokens !== null) {
      parts.push(`output ${formatNumber(providerUsage.outputTokens)}`);
    }
    return parts.length > 0
      ? `Provider-reported usage (latest completed request): ${parts.join(" · ")}`
      : "Provider-reported usage unavailable for latest request";
  });

  // Pressure states
  let pressureLevel = $derived(
    percentUsed >= 95 ? 'critical' :
    percentUsed >= 90 ? 'urgent' :
    percentUsed >= 80 ? 'warning' :
    percentUsed >= 60 ? 'attention' : 'calm'
  );

  function formatNumber(n: number): string {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + "M";
    if (n >= 1000) return (n / 1000).toFixed(1) + "k";
    return n.toLocaleString();
  }
</script>

<div class="budget-bar" data-pressure={pressureLevel}>
  <div class="budget-track">
    <div class="budget-fill" style:width="{percentUsed}%">
      <div class="budget-dither"></div>
    </div>
    <div class="budget-markers">
      <div class="marker" style:left="60%"></div>
      <div class="marker" style:left="80%"></div>
      <div class="marker marker-danger" style:left="90%"></div>
      <div
        class="marker marker-threshold marker-soft"
        style:left="{plannerThresholds.soft * 100}%"
        title="Soft threshold: {(plannerThresholds.soft * 100).toFixed(0)}%"
      ></div>
      <div
        class="marker marker-threshold marker-medium"
        style:left="{plannerThresholds.medium * 100}%"
        title="Medium threshold: {(plannerThresholds.medium * 100).toFixed(0)}%"
      ></div>
      <div
        class="marker marker-threshold marker-hard"
        style:left="{plannerThresholds.hard * 100}%"
        title="Hard threshold: {(plannerThresholds.hard * 100).toFixed(0)}%"
      ></div>
      <div
        class="marker marker-ceiling"
        style:left="{plannerThresholds.ceiling * 100}%"
        title="Budget ceiling: {(plannerThresholds.ceiling * 100).toFixed(0)}%"
      ></div>
    </div>
  </div>
  <div class="budget-stats">
    <span class="stat stat-used">
      <span class="stat-value">{formatNumber(budget.used)}</span>
      <span class="stat-label">used</span>
    </span>
    <span class="stat-sep">/</span>
    <span class="stat stat-free">
      <span class="stat-value">{formatNumber(freeTokens)}</span>
      <span class="stat-label">free</span>
    </span>
    <span class="stat stat-percent">
      <span class="stat-value">{percentUsed.toFixed(0)}%</span>
    </span>
    {#if providerTotal !== null}
      <span class="stat-sep">•</span>
      <span class="stat stat-provider" title={providerTitle}>
        <span class="stat-value">{formatNumber(providerTotal)}</span>
        <span class="stat-label">provider</span>
      </span>
    {/if}
  </div>
</div>

<style>
  .budget-bar {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .budget-track {
    flex: 1;
    height: 8px;
    background: var(--bg-inset);
    border-radius: 4px;
    overflow: hidden;
    position: relative;
    border: 1px solid var(--border-subtle);
  }

  .budget-fill {
    height: 100%;
    background: var(--text-primary);
    border-radius: 3px;
    transition: width 0.3s ease, background 0.3s ease;
    position: relative;
    overflow: hidden;
  }

  /* Dither overlay on fill */
  .budget-dither {
    position: absolute;
    inset: 0;
    opacity: 0.15;
    background-image: url("data:image/svg+xml,%3Csvg width='4' height='4' viewBox='0 0 4 4' xmlns='http://www.w3.org/2000/svg'%3E%3Ccircle cx='1' cy='1' r='0.5' fill='%23fff'/%3E%3Ccircle cx='3' cy='3' r='0.5' fill='%23fff'/%3E%3C/svg%3E");
    background-size: 4px 4px;
  }

  /* Threshold markers */
  .budget-markers {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .marker {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--border-default);
    opacity: 0.5;
  }

  .marker-danger {
    background: var(--semantic-danger);
    opacity: 0.4;
  }

  .marker-threshold {
    width: 1px;
    opacity: 0.3;
    border-style: dotted;
  }

  .marker-soft {
    background: var(--zone-middle);
  }

  .marker-medium {
    background: var(--semantic-warning);
  }

  .marker-hard {
    background: var(--semantic-danger);
  }

  .marker-ceiling {
    width: 2px;
    background: var(--accent);
    opacity: 0.7;
    z-index: 1;
  }

  /* Pressure colors */
  [data-pressure="attention"] .budget-fill {
    background: var(--zone-middle);
  }

  [data-pressure="warning"] .budget-fill {
    background: var(--semantic-warning);
  }

  [data-pressure="urgent"] .budget-fill,
  [data-pressure="critical"] .budget-fill {
    background: var(--semantic-danger);
  }

  [data-pressure="urgent"] .budget-fill {
    animation: budget-pulse 1.8s ease-in-out infinite;
  }

  [data-pressure="critical"] .budget-fill {
    animation: budget-pulse 1s ease-in-out infinite;
  }

  @keyframes budget-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
  }

  .budget-stats {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .stat {
    display: flex;
    align-items: baseline;
    gap: 3px;
    font-family: var(--font-mono);
  }

  .stat-sep {
    color: var(--text-faint);
    font-size: 10px;
  }

  .stat-value {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 9px;
    color: var(--text-muted);
  }

  .stat-percent .stat-value {
    font-size: 10px;
    color: var(--text-muted);
    font-weight: 400;
  }

  .stat-provider .stat-value {
    color: var(--accent);
    font-weight: 600;
  }

  .stat-provider .stat-label {
    color: var(--text-faint);
  }

  [data-pressure="urgent"] .stat-used .stat-value,
  [data-pressure="critical"] .stat-used .stat-value {
    color: var(--semantic-danger);
  }
</style>
