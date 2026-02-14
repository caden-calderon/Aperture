<script lang="ts">
  import { contextStore } from "$lib/stores";
  import { plannerThresholdsFromCeilingPercent } from "$lib/utils/planner-thresholds";

  interface Props {
    open: boolean;
    onclose: () => void;
  }

  let { open, onclose }: Props = $props();

  let ceiling = $state(contextStore.budgetCeiling * 100);
  let compression = $state(contextStore.compressionSettings);

  // Derived threshold markers from ceiling
  let thresholds = $derived(plannerThresholdsFromCeilingPercent(ceiling));

  // Stats from context store
  let totalBlocks = $derived(contextStore.blocks.length);
  let budget = $derived(contextStore.tokenBudget);

  async function handleCeilingChange(e: Event) {
    const value = parseFloat((e.target as HTMLInputElement).value);
    ceiling = value;
    await contextStore.setBudgetCeiling(value / 100);
  }

  async function handleCompressionBackendChange(e: Event) {
    const backend = (e.target as HTMLSelectElement).value as typeof compression.backend;
    compression = { ...compression, backend };
    await contextStore.updateCompressionSettings({ backend });
  }

  async function handleCompressionModelChange(e: Event) {
    const modelRaw = (e.target as HTMLInputElement).value.trim();
    const model = modelRaw.length > 0 ? modelRaw : null;
    compression = { ...compression, model };
    await contextStore.updateCompressionSettings({ model });
  }

  async function handleCompressionTimeoutChange(e: Event) {
    const timeoutMs = Math.round(parseFloat((e.target as HTMLInputElement).value));
    compression = { ...compression, timeoutMs };
    await contextStore.updateCompressionSettings({ timeoutMs });
  }

  async function handleCompressionMaxTokensChange(e: Event) {
    const maxTokens = Math.round(parseFloat((e.target as HTMLInputElement).value));
    compression = { ...compression, maxTokens };
    await contextStore.updateCompressionSettings({ maxTokens });
  }

  // Sync from store if externally changed
  $effect(() => {
    ceiling = contextStore.budgetCeiling * 100;
    compression = contextStore.compressionSettings;
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="settings-backdrop" onmousedown={onclose}></div>
  <div class="settings-panel">
    <div class="settings-header">
      <span class="settings-title">Settings</span>
      <button
        class="settings-close"
        onclick={onclose}
        aria-label="Close settings panel"
        title="Close settings"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M18 6L6 18M6 6l12 12" />
        </svg>
      </button>
    </div>

    <div class="settings-body">
      <!-- Budget Ceiling -->
      <section class="settings-section">
        <h3 class="section-title">Budget Ceiling</h3>
        <div class="ceiling-control">
          <input
            type="range"
            min="40"
            max="100"
            step="5"
            value={ceiling}
            oninput={handleCeilingChange}
            class="ceiling-slider"
          />
          <span class="ceiling-value">{ceiling.toFixed(0)}%</span>
        </div>
        <div class="threshold-markers">
          <span class="threshold" data-level="soft">soft {thresholds.soft}%</span>
          <span class="threshold" data-level="medium">medium {thresholds.medium}%</span>
          <span class="threshold" data-level="hard">hard {thresholds.hard}%</span>
        </div>
        <p class="section-hint">
          Heuristic archival triggers when budget exceeds these thresholds.
        </p>
      </section>

      <!-- Context Stats -->
      <section class="settings-section">
        <h3 class="section-title">Context Stats</h3>
        <div class="stats-grid">
          <div class="stat-item">
            <span class="stat-num">{totalBlocks}</span>
            <span class="stat-lbl">blocks</span>
          </div>
          <div class="stat-item">
            <span class="stat-num">{budget.used.toLocaleString()}</span>
            <span class="stat-lbl">tokens used</span>
          </div>
          <div class="stat-item">
            <span class="stat-num">{Math.round((budget.used / Math.max(budget.limit, 1)) * 100)}%</span>
            <span class="stat-lbl">utilization</span>
          </div>
        </div>
      </section>

      <!-- Compression Sidekick -->
      <section class="settings-section">
        <h3 class="section-title">Compression Sidekick</h3>
        <div class="field">
          <label class="field-label" for="compression-backend">Backend</label>
          <select
            id="compression-backend"
            class="field-control"
            value={compression.backend}
            onchange={handleCompressionBackendChange}
          >
            <option value="auto">Auto (provider-aware)</option>
            <option value="anthropic">Anthropic</option>
            <option value="open_ai">OpenAI</option>
            <option value="open_router">OpenRouter</option>
            <option value="disabled">Disabled (fail-open)</option>
          </select>
        </div>
        <div class="field">
          <label class="field-label" for="compression-model">Model Override</label>
          <input
            id="compression-model"
            class="field-control"
            type="text"
            value={compression.model ?? ""}
            placeholder="default per backend"
            onblur={handleCompressionModelChange}
          />
        </div>
        <div class="field-grid">
          <div class="field">
            <label class="field-label" for="compression-timeout">Timeout (ms)</label>
            <input
              id="compression-timeout"
              class="field-control"
              type="number"
              min="500"
              max="120000"
              step="500"
              value={compression.timeoutMs}
              onchange={handleCompressionTimeoutChange}
            />
          </div>
          <div class="field">
            <label class="field-label" for="compression-max-tokens">Max Tokens</label>
            <input
              id="compression-max-tokens"
              class="field-control"
              type="number"
              min="64"
              max="8192"
              step="32"
              value={compression.maxTokens}
              onchange={handleCompressionMaxTokensChange}
            />
          </div>
        </div>
        <p class="section-hint">
          Sidekick compression is fail-open. If unavailable, existing planner behavior is preserved.
        </p>
      </section>
    </div>
  </div>
{/if}

<style>
  .settings-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
  }

  .settings-panel {
    position: fixed;
    top: 32px;
    right: 8px;
    width: 280px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-default);
    border-radius: 6px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    z-index: 100;
    font-family: var(--font-mono);
    max-height: calc(100vh - 48px);
    overflow-y: auto;
  }

  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .settings-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .settings-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
  }

  .settings-close:hover {
    color: var(--text-primary);
  }

  .settings-body {
    padding: 8px 0;
  }

  .settings-section {
    padding: 8px 12px;
  }

  .settings-section + .settings-section {
    border-top: 1px solid var(--border-subtle);
  }

  .section-title {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    letter-spacing: 0.3px;
    text-transform: uppercase;
    margin: 0 0 8px 0;
  }

  .section-hint {
    font-size: 9px;
    color: var(--text-faint);
    margin: 6px 0 0 0;
    line-height: 1.4;
  }

  /* Budget Ceiling */
  .ceiling-control {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ceiling-slider {
    flex: 1;
    height: 4px;
    appearance: none;
    background: var(--bg-inset);
    border-radius: 2px;
    outline: none;
    cursor: pointer;
  }

  .ceiling-slider::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent);
    border: 2px solid var(--bg-elevated);
    cursor: pointer;
  }

  .ceiling-value {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    min-width: 36px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .threshold-markers {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }

  .threshold {
    font-size: 9px;
    padding: 1px 4px;
    border-radius: 3px;
    font-variant-numeric: tabular-nums;
  }

  .threshold[data-level="soft"] {
    color: var(--zone-middle);
    background: color-mix(in srgb, var(--zone-middle) 15%, transparent);
  }

  .threshold[data-level="medium"] {
    color: var(--semantic-warning);
    background: color-mix(in srgb, var(--semantic-warning) 15%, transparent);
  }

  .threshold[data-level="hard"] {
    color: var(--semantic-danger);
    background: color-mix(in srgb, var(--semantic-danger) 15%, transparent);
  }

  /* Stats Grid */
  .stats-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }

  .stat-num {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-lbl {
    font-size: 8px;
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 8px;
  }

  .field-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .field-label {
    font-size: 9px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.2px;
  }

  .field-control {
    width: 100%;
    background: var(--bg-inset);
    border: 1px solid var(--border-default);
    color: var(--text-primary);
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 11px;
    font-family: var(--font-mono);
  }
</style>
