/**
 * Theme Store - Customizable color themes with presets and persistence
 */

// ============================================================================
// Types
// ============================================================================

export interface ThemeColors {
  // Background colors
  bgBase: string;
  bgSurface: string;
  bgElevated: string;
  bgHover: string;
  bgMuted: string;
  // Border colors
  borderSubtle: string;
  borderDefault: string;
  // Text colors
  textPrimary: string;
  textSecondary: string;
  textMuted: string;
  // Accent
  accent: string;
  // Role/block type colors
  roleSystem: string;
  roleUser: string;
  roleAssistant: string;
  roleTool: string;
  roleThinking: string;
  // Semantic colors
  semanticDanger: string;
  semanticWarning: string;
  semanticSuccess: string;
}

export interface ThemePreset {
  id: string;
  name: string;
  isDark: boolean;
  colors: ThemeColors;
  isCustom?: boolean;
}

// ============================================================================
// Built-in Presets
// ============================================================================

export const PRESETS: ThemePreset[] = [
  // === DARK THEMES ===
  {
    id: 'dark-charcoal',
    name: 'Charcoal',
    isDark: true,
    colors: {
      bgBase: '#1a1816',
      bgSurface: '#24201d',
      bgElevated: '#2e2a26',
      bgHover: '#3a3530',
      bgMuted: '#4a4540',
      borderSubtle: '#3a3530',
      borderDefault: '#4a4540',
      textPrimary: '#f5f5dc',
      textSecondary: '#ded1b6',
      textMuted: '#a89880',
      accent: '#f5f5dc',
      roleSystem: '#9080a8',
      roleUser: '#7090b0',
      roleAssistant: '#80a880',
      roleTool: '#b0a080',
      roleThinking: '#9882c4',
      semanticDanger: '#c86860',
      semanticWarning: '#c8a060',
      semanticSuccess: '#70a870',
    },
  },
  {
    id: 'tokyo-night',
    name: 'Tokyo Night',
    isDark: true,
    colors: {
      bgBase: '#1a1b26',
      bgSurface: '#24283b',
      bgElevated: '#2f3549',
      bgHover: '#3b4261',
      bgMuted: '#414868',
      borderSubtle: '#3b4261',
      borderDefault: '#414868',
      textPrimary: '#c0caf5',
      textSecondary: '#a9b1d6',
      textMuted: '#565f89',
      accent: '#7aa2f7',
      roleSystem: '#bb9af7',
      roleUser: '#7aa2f7',
      roleAssistant: '#9ece6a',
      roleTool: '#e0af68',
      roleThinking: '#a78bfa',
      semanticDanger: '#f7768e',
      semanticWarning: '#e0af68',
      semanticSuccess: '#9ece6a',
    },
  },
  {
    id: 'gruvbox-dark',
    name: 'Gruvbox Dark',
    isDark: true,
    colors: {
      bgBase: '#282828',
      bgSurface: '#3c3836',
      bgElevated: '#504945',
      bgHover: '#665c54',
      bgMuted: '#7c6f64',
      borderSubtle: '#504945',
      borderDefault: '#665c54',
      textPrimary: '#ebdbb2',
      textSecondary: '#d5c4a1',
      textMuted: '#a89984',
      accent: '#fe8019',
      roleSystem: '#d3869b',
      roleUser: '#83a598',
      roleAssistant: '#b8bb26',
      roleTool: '#fabd2f',
      roleThinking: '#b48ead',
      semanticDanger: '#fb4934',
      semanticWarning: '#fabd2f',
      semanticSuccess: '#b8bb26',
    },
  },
  {
    id: 'catppuccin-mocha',
    name: 'Catppuccin',
    isDark: true,
    colors: {
      bgBase: '#1e1e2e',
      bgSurface: '#313244',
      bgElevated: '#45475a',
      bgHover: '#585b70',
      bgMuted: '#6c7086',
      borderSubtle: '#45475a',
      borderDefault: '#585b70',
      textPrimary: '#cdd6f4',
      textSecondary: '#bac2de',
      textMuted: '#a6adc8',
      accent: '#cba6f7',
      roleSystem: '#cba6f7',
      roleUser: '#89b4fa',
      roleAssistant: '#a6e3a1',
      roleTool: '#f9e2af',
      roleThinking: '#b4a0e5',
      semanticDanger: '#f38ba8',
      semanticWarning: '#fab387',
      semanticSuccess: '#a6e3a1',
    },
  },
  {
    id: 'nord',
    name: 'Nord',
    isDark: true,
    colors: {
      bgBase: '#2e3440',
      bgSurface: '#3b4252',
      bgElevated: '#434c5e',
      bgHover: '#4c566a',
      bgMuted: '#5e6779',
      borderSubtle: '#434c5e',
      borderDefault: '#4c566a',
      textPrimary: '#eceff4',
      textSecondary: '#e5e9f0',
      textMuted: '#a3be8c',
      accent: '#88c0d0',
      roleSystem: '#b48ead',
      roleUser: '#81a1c1',
      roleAssistant: '#a3be8c',
      roleTool: '#ebcb8b',
      roleThinking: '#9b8ec4',
      semanticDanger: '#bf616a',
      semanticWarning: '#ebcb8b',
      semanticSuccess: '#a3be8c',
    },
  },
  {
    id: 'dracula',
    name: 'Dracula',
    isDark: true,
    colors: {
      bgBase: '#282a36',
      bgSurface: '#343746',
      bgElevated: '#44475a',
      bgHover: '#4d5066',
      bgMuted: '#5a5d73',
      borderSubtle: '#44475a',
      borderDefault: '#5a5d73',
      textPrimary: '#f8f8f2',
      textSecondary: '#e6e6dc',
      textMuted: '#6272a4',
      accent: '#bd93f9',
      roleSystem: '#bd93f9',
      roleUser: '#8be9fd',
      roleAssistant: '#50fa7b',
      roleTool: '#ffb86c',
      roleThinking: '#c4a7fa',
      semanticDanger: '#ff5555',
      semanticWarning: '#ffb86c',
      semanticSuccess: '#50fa7b',
    },
  },
  {
    id: 'one-dark',
    name: 'One Dark',
    isDark: true,
    colors: {
      bgBase: '#282c34',
      bgSurface: '#2c323c',
      bgElevated: '#3a3f4b',
      bgHover: '#4b5263',
      bgMuted: '#5c6370',
      borderSubtle: '#3a3f4b',
      borderDefault: '#4b5263',
      textPrimary: '#abb2bf',
      textSecondary: '#9da5b4',
      textMuted: '#5c6370',
      accent: '#61afef',
      roleSystem: '#c678dd',
      roleUser: '#61afef',
      roleAssistant: '#98c379',
      roleTool: '#e5c07b',
      roleThinking: '#b07ed8',
      semanticDanger: '#e06c75',
      semanticWarning: '#e5c07b',
      semanticSuccess: '#98c379',
    },
  },
  {
    id: 'solarized-dark',
    name: 'Solarized',
    isDark: true,
    colors: {
      bgBase: '#002b36',
      bgSurface: '#073642',
      bgElevated: '#0a4050',
      bgHover: '#0d4a5a',
      bgMuted: '#1a5a6a',
      borderSubtle: '#0d4a5a',
      borderDefault: '#1a5a6a',
      textPrimary: '#fdf6e3',
      textSecondary: '#eee8d5',
      textMuted: '#93a1a1',
      accent: '#2aa198',
      roleSystem: '#d33682',
      roleUser: '#268bd2',
      roleAssistant: '#859900',
      roleTool: '#b58900',
      roleThinking: '#8b7ec8',
      semanticDanger: '#dc322f',
      semanticWarning: '#cb4b16',
      semanticSuccess: '#859900',
    },
  },
  // === LIGHT THEMES ===
  {
    id: 'light-warm',
    name: 'Warm',
    isDark: false,
    colors: {
      bgBase: '#e8e0d0',
      bgSurface: '#f5f0e8',
      bgElevated: '#ebe5da',
      bgHover: '#ddd5c5',
      bgMuted: '#d0c8b8',
      borderSubtle: '#ccc4b4',
      borderDefault: '#b8b0a0',
      textPrimary: '#2a2520',
      textSecondary: '#4a4540',
      textMuted: '#6a6560',
      accent: '#2a2520',
      roleSystem: '#6a4878',
      roleUser: '#3a5878',
      roleAssistant: '#3a6848',
      roleTool: '#705838',
      roleThinking: '#6a4b8a',
      semanticDanger: '#a04848',
      semanticWarning: '#907030',
      semanticSuccess: '#3a6838',
    },
  },
  {
    id: 'gruvbox-light',
    name: 'Gruvbox Light',
    isDark: false,
    colors: {
      bgBase: '#fbf1c7',
      bgSurface: '#f9f5d7',
      bgElevated: '#f2e5bc',
      bgHover: '#ebdbb2',
      bgMuted: '#d5c4a1',
      borderSubtle: '#d5c4a1',
      borderDefault: '#bdae93',
      textPrimary: '#3c3836',
      textSecondary: '#504945',
      textMuted: '#7c6f64',
      accent: '#d65d0e',
      roleSystem: '#8f3f71',
      roleUser: '#076678',
      roleAssistant: '#79740e',
      roleTool: '#b57614',
      roleThinking: '#7c4d8a',
      semanticDanger: '#9d0006',
      semanticWarning: '#b57614',
      semanticSuccess: '#79740e',
    },
  },
  {
    id: 'tokyo-night-light',
    name: 'Tokyo Light',
    isDark: false,
    colors: {
      bgBase: '#d5d6db',
      bgSurface: '#e9e9ec',
      bgElevated: '#dddee1',
      bgHover: '#c8c9ce',
      bgMuted: '#b8b9be',
      borderSubtle: '#c8c9ce',
      borderDefault: '#9699a3',
      textPrimary: '#343b58',
      textSecondary: '#4c505e',
      textMuted: '#6c6f7e',
      accent: '#2e7de9',
      roleSystem: '#9854f1',
      roleUser: '#2e7de9',
      roleAssistant: '#587539',
      roleTool: '#8a5b0e',
      roleThinking: '#7847a0',
      semanticDanger: '#c64343',
      semanticWarning: '#8a5b0e',
      semanticSuccess: '#587539',
    },
  },
  {
    id: 'light-sepia',
    name: 'Sepia',
    isDark: false,
    colors: {
      bgBase: '#f4ecd8',
      bgSurface: '#faf6ec',
      bgElevated: '#f0e8d4',
      bgHover: '#e8dcc4',
      bgMuted: '#dcd0b8',
      borderSubtle: '#d4c8b0',
      borderDefault: '#c0b498',
      textPrimary: '#3a3020',
      textSecondary: '#5a5040',
      textMuted: '#7a7060',
      accent: '#6a5030',
      roleSystem: '#704878',
      roleUser: '#406080',
      roleAssistant: '#486848',
      roleTool: '#806030',
      roleThinking: '#6e4a82',
      semanticDanger: '#904040',
      semanticWarning: '#906820',
      semanticSuccess: '#406838',
    },
  },
  {
    id: 'solarized-light',
    name: 'Solarized Lt',
    isDark: false,
    colors: {
      bgBase: '#fdf6e3',
      bgSurface: '#fffbf0',
      bgElevated: '#f5eed8',
      bgHover: '#eee8d0',
      bgMuted: '#e0dac8',
      borderSubtle: '#d8d2c0',
      borderDefault: '#c8c2b0',
      textPrimary: '#073642',
      textSecondary: '#586e75',
      textMuted: '#93a1a1',
      accent: '#268bd2',
      roleSystem: '#d33682',
      roleUser: '#268bd2',
      roleAssistant: '#859900',
      roleTool: '#b58900',
      roleThinking: '#7c5caa',
      semanticDanger: '#dc322f',
      semanticWarning: '#cb4b16',
      semanticSuccess: '#859900',
    },
  },
];

// ============================================================================
// State
// ============================================================================

let currentPresetId = $state<string>('dark-charcoal');
let customColors = $state<Partial<ThemeColors>>({});
let savedThemes = $state<ThemePreset[]>([]);

// ============================================================================
// Helpers
// ============================================================================

function getPresetById(id: string): ThemePreset | undefined {
  return PRESETS.find(p => p.id === id) || savedThemes.find(p => p.id === id);
}

function adjustBrightness(hex: string, amount: number): string {
  const num = parseInt(hex.replace('#', ''), 16);
  const r = Math.min(255, Math.max(0, ((num >> 16) & 0xff) + Math.round(255 * amount)));
  const g = Math.min(255, Math.max(0, ((num >> 8) & 0xff) + Math.round(255 * amount)));
  const b = Math.min(255, Math.max(0, (num & 0xff) + Math.round(255 * amount)));
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`;
}

function hexToRgba(hex: string, alpha: number): string {
  const num = parseInt(hex.replace('#', ''), 16);
  const r = (num >> 16) & 0xff;
  const g = (num >> 8) & 0xff;
  const b = num & 0xff;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

// ============================================================================
// Derived
// ============================================================================

function getCurrentPreset(): ThemePreset {
  return getPresetById(currentPresetId) || PRESETS[0];
}

function getEffectiveColors(): ThemeColors {
  const preset = getCurrentPreset();
  return { ...preset.colors, ...customColors };
}

function getIsDark(): boolean {
  return getCurrentPreset().isDark;
}

// ============================================================================
// Actions
// ============================================================================

function applyColors(colors: ThemeColors, isDark: boolean): void {
  if (typeof document === 'undefined') return;

  const root = document.documentElement;

  // Background colors
  root.style.setProperty('--bg-base', colors.bgBase);
  root.style.setProperty('--bg-surface', colors.bgSurface);
  root.style.setProperty('--bg-elevated', colors.bgElevated);
  root.style.setProperty('--bg-hover', colors.bgHover);
  root.style.setProperty('--bg-muted', colors.bgMuted);
  root.style.setProperty('--bg-inset', adjustBrightness(colors.bgBase, isDark ? -0.02 : -0.03));

  // Border colors
  root.style.setProperty('--border-subtle', colors.borderSubtle);
  root.style.setProperty('--border-default', colors.borderDefault);
  root.style.setProperty('--border-strong', adjustBrightness(colors.borderDefault, isDark ? 0.1 : -0.15));

  // Text colors
  root.style.setProperty('--text-primary', colors.textPrimary);
  root.style.setProperty('--text-secondary', colors.textSecondary);
  root.style.setProperty('--text-muted', colors.textMuted);
  root.style.setProperty('--text-faint', adjustBrightness(colors.textMuted, isDark ? -0.15 : 0.15));

  // Accent colors
  root.style.setProperty('--accent', colors.accent);
  root.style.setProperty('--accent-subtle', hexToRgba(colors.accent, 0.08));
  root.style.setProperty('--accent-muted', hexToRgba(colors.accent, 0.16));

  // Role/block type colors
  root.style.setProperty('--role-system', colors.roleSystem);
  root.style.setProperty('--role-user', colors.roleUser);
  root.style.setProperty('--role-assistant', colors.roleAssistant);
  root.style.setProperty('--role-tool', colors.roleTool);
  root.style.setProperty('--role-thinking', colors.roleThinking);

  // Semantic colors
  root.style.setProperty('--semantic-danger', colors.semanticDanger);
  root.style.setProperty('--semantic-warning', colors.semanticWarning);
  root.style.setProperty('--semantic-success', colors.semanticSuccess);

  root.setAttribute('data-theme', isDark ? 'dark' : 'light');
}

function setPreset(presetId: string): void {
  const preset = getPresetById(presetId);
  if (preset) {
    currentPresetId = preset.id;
    customColors = {};
    applyColors(preset.colors, preset.isDark);
    saveToLocalStorage();
  }
}

function setColor(key: keyof ThemeColors, value: string): void {
  customColors = { ...customColors, [key]: value };
  applyColors(getEffectiveColors(), getIsDark());
  saveToLocalStorage();
}

function resetCustomColors(): void {
  customColors = {};
  const preset = getCurrentPreset();
  applyColors(preset.colors, preset.isDark);
  saveToLocalStorage();
}

function saveCurrentAsPreset(name: string): string {
  const id = `custom-${Date.now()}`;
  const newPreset: ThemePreset = {
    id,
    name,
    isDark: getIsDark(),
    colors: { ...getEffectiveColors() },
    isCustom: true,
  };
  savedThemes = [...savedThemes, newPreset];
  currentPresetId = id;
  saveToLocalStorage();
  return id;
}

function deleteCustomPreset(id: string): void {
  savedThemes = savedThemes.filter(p => p.id !== id);
  if (currentPresetId === id) {
    setPreset('dark-charcoal');
  }
  saveToLocalStorage();
}

function saveToLocalStorage(): void {
  if (typeof localStorage === 'undefined') return;
  const data = {
    currentPresetId,
    customColors,
    savedThemes,
  };
  localStorage.setItem('aperture-theme-data', JSON.stringify(data));
}

function loadFromLocalStorage(): void {
  if (typeof localStorage === 'undefined') return;
  try {
    const stored = localStorage.getItem('aperture-theme-data');
    if (stored) {
      const data = JSON.parse(stored);
      if (data.currentPresetId) currentPresetId = data.currentPresetId;
      if (data.customColors) customColors = data.customColors;
      if (data.savedThemes) savedThemes = data.savedThemes;
    }
  } catch (e) {
    console.error('Failed to load theme data:', e);
  }
}

function init(): void {
  loadFromLocalStorage();
  applyColors(getEffectiveColors(), getIsDark());
}

// Legacy toggle between dark and light
function toggle(): void {
  if (getIsDark()) {
    const lightPreset = PRESETS.find(p => !p.isDark);
    if (lightPreset) setPreset(lightPreset.id);
  } else {
    const darkPreset = PRESETS.find(p => p.isDark);
    if (darkPreset) setPreset(darkPreset.id);
  }
}

function set(mode: 'light' | 'dark'): void {
  if (mode === 'dark' && !getIsDark()) {
    const darkPreset = PRESETS.find(p => p.isDark);
    if (darkPreset) setPreset(darkPreset.id);
  } else if (mode === 'light' && getIsDark()) {
    const lightPreset = PRESETS.find(p => !p.isDark);
    if (lightPreset) setPreset(lightPreset.id);
  }
}

// ============================================================================
// xterm.js Theme Mapping
// ============================================================================

export interface XtermTheme {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  selectionForeground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

function getXtermTheme(): XtermTheme {
  const c = getEffectiveColors();
  return {
    background: c.bgBase,
    foreground: c.textPrimary,
    cursor: c.textPrimary,
    cursorAccent: c.bgBase,
    selectionBackground: c.bgHover,
    selectionForeground: c.textPrimary,
    black: c.bgBase,
    red: c.semanticDanger,
    green: c.semanticSuccess,
    yellow: c.semanticWarning,
    blue: c.roleUser,
    magenta: c.roleSystem,
    cyan: c.accent,
    white: c.textPrimary,
    brightBlack: c.textMuted,
    brightRed: c.semanticDanger,
    brightGreen: c.semanticSuccess,
    brightYellow: c.semanticWarning,
    brightBlue: c.roleUser,
    brightMagenta: c.roleSystem,
    brightCyan: c.accent,
    brightWhite: c.textPrimary,
  };
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const themeStore = {
  get currentPresetId() { return currentPresetId; },
  get currentPreset() { return getCurrentPreset(); },
  get customColors() { return customColors; },
  get effectiveColors() { return getEffectiveColors(); },
  get isDark() { return getIsDark(); },
  get savedThemes() { return savedThemes; },
  get allPresets() { return [...PRESETS, ...savedThemes]; },
  get builtInPresets() { return PRESETS; },

  init,
  setPreset,
  setColor,
  resetCustomColors,
  saveCurrentAsPreset,
  deleteCustomPreset,
  toggle,
  set,
  getXtermTheme,
};
