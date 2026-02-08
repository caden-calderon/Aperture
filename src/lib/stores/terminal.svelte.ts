/**
 * Terminal Store - Terminal panel state and persistence
 */

// ============================================================================
// Types
// ============================================================================

export type TerminalPosition = 'bottom' | 'right';
export type TerminalProvider = 'anthropic' | 'openai' | 'openai_proxy' | 'none';
export type LaunchStatus = 'idle' | 'launching' | 'running' | 'error';

// ============================================================================
// State
// ============================================================================

let sessionId = $state<string | null>(null);
let isVisible = $state(false);
let terminalHeight = $state(300);
let terminalWidth = $state(400);
let position = $state<TerminalPosition>('bottom');
let isExited = $state(false);
let preCollapseHeight = $state(300);
let preCollapseWidth = $state(400);
let selectedProvider = $state<TerminalProvider>('none');
let launchStatus = $state<LaunchStatus>('idle');

// ============================================================================
// Persistence Keys
// ============================================================================

const STORAGE_HEIGHT = 'aperture-terminal-height';
const STORAGE_WIDTH = 'aperture-terminal-width';
const STORAGE_VISIBLE = 'aperture-terminal-visible';
const STORAGE_POSITION = 'aperture-terminal-position';

// ============================================================================
// Actions
// ============================================================================

function show(): void {
  isVisible = true;
  saveVisibility();
}

function hide(): void {
  isVisible = false;
  saveVisibility();
}

function toggle(): void {
  isVisible = !isVisible;
  saveVisibility();
}

// Collapsed bar size — thin bar with icon
const COLLAPSED_SIZE = 28;
// Minimum usable size — anything below this snaps to collapsed
const MIN_USABLE_HEIGHT = 120;
const MIN_USABLE_WIDTH = 180;

function setHeight(px: number): void {
  // Snap to collapsed if below minimum usable size
  if (px < MIN_USABLE_HEIGHT) {
    // Save current height before collapsing (only if currently expanded)
    if (terminalHeight >= MIN_USABLE_HEIGHT) {
      preCollapseHeight = terminalHeight;
    }
    terminalHeight = COLLAPSED_SIZE;
  } else {
    terminalHeight = px;
  }
  saveHeight();
}

function setWidth(px: number): void {
  if (px < MIN_USABLE_WIDTH) {
    if (terminalWidth >= MIN_USABLE_WIDTH) {
      preCollapseWidth = terminalWidth;
    }
    terminalWidth = COLLAPSED_SIZE;
  } else {
    terminalWidth = px;
  }
  saveWidth();
}

function collapseTerminal(): void {
  if (position === 'bottom') {
    setHeight(0); // Triggers snap to COLLAPSED_SIZE
  } else {
    setWidth(0);
  }
}

function expandFromCollapsed(): void {
  if (position === 'bottom') {
    setHeight(preCollapseHeight);
  } else {
    setWidth(preCollapseWidth);
  }
}

function toggleCollapsed(): void {
  const size = position === 'bottom' ? terminalHeight : terminalWidth;
  if (size <= COLLAPSED_SIZE) {
    expandFromCollapsed();
  } else {
    collapseTerminal();
  }
}

function setPosition(pos: TerminalPosition): void {
  position = pos;
  savePosition();
}

function togglePosition(): void {
  position = position === 'bottom' ? 'right' : 'bottom';
  savePosition();
}

function setSessionId(id: string | null): void {
  sessionId = id;
}

function setExited(value: boolean): void {
  isExited = value;
}

function setSelectedProvider(provider: TerminalProvider): void {
  selectedProvider = provider;
}

function setLaunchStatus(s: LaunchStatus): void {
  launchStatus = s;
}

// ============================================================================
// Persistence
// ============================================================================

function saveVisibility(): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_VISIBLE, JSON.stringify(isVisible));
}

function saveHeight(): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_HEIGHT, JSON.stringify(terminalHeight));
}

function saveWidth(): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_WIDTH, JSON.stringify(terminalWidth));
}

function savePosition(): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_POSITION, JSON.stringify(position));
}

function init(): void {
  if (typeof localStorage === 'undefined') return;
  try {
    const h = localStorage.getItem(STORAGE_HEIGHT);
    if (h) terminalHeight = JSON.parse(h);

    const w = localStorage.getItem(STORAGE_WIDTH);
    if (w) terminalWidth = JSON.parse(w);

    const v = localStorage.getItem(STORAGE_VISIBLE);
    if (v) isVisible = JSON.parse(v);

    const p = localStorage.getItem(STORAGE_POSITION);
    if (p) position = JSON.parse(p);
  } catch (e) {
    console.error('Failed to load terminal state:', e);
  }
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const terminalStore = {
  get sessionId() { return sessionId; },
  get isVisible() { return isVisible; },
  get terminalHeight() { return terminalHeight; },
  get terminalWidth() { return terminalWidth; },
  get position() { return position; },
  get isExited() { return isExited; },
  get isCollapsed() {
    const size = position === 'bottom' ? terminalHeight : terminalWidth;
    return size <= COLLAPSED_SIZE;
  },
  get collapsedSize() { return COLLAPSED_SIZE; },
  get selectedProvider() { return selectedProvider; },
  get launchStatus() { return launchStatus; },

  init,
  show,
  hide,
  toggle,
  setHeight,
  setWidth,
  setPosition,
  togglePosition,
  setSessionId,
  setExited,
  setSelectedProvider,
  setLaunchStatus,
  collapseTerminal,
  expandFromCollapsed,
  toggleCollapsed,
};
