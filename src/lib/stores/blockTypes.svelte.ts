/**
 * Block Types Store
 * Manages built-in and custom block types with their colors
 */

// ============================================================================
// Types
// ============================================================================

export interface BlockType {
  id: string;
  label: string;
  shortLabel: string;
  color: string;
  isBuiltIn: boolean;
}

// ============================================================================
// Built-in Types
// ============================================================================

const BUILT_IN_TYPES: BlockType[] = [
  {
    id: 'system',
    label: 'System',
    shortLabel: 'SYS',
    color: 'var(--role-system)',
    isBuiltIn: true,
  },
  {
    id: 'user',
    label: 'User',
    shortLabel: 'USR',
    color: 'var(--role-user)',
    isBuiltIn: true,
  },
  {
    id: 'assistant',
    label: 'Assistant',
    shortLabel: 'AST',
    color: 'var(--role-assistant)',
    isBuiltIn: true,
  },
  {
    id: 'tool_use',
    label: 'Tool Use',
    shortLabel: 'TOOL',
    color: 'var(--role-tool)',
    isBuiltIn: true,
  },
  {
    id: 'tool_result',
    label: 'Tool Result',
    shortLabel: 'RES',
    color: 'var(--role-tool)',
    isBuiltIn: true,
  },
  {
    id: 'thinking',
    label: 'Thinking',
    shortLabel: 'THINK',
    color: 'var(--role-thinking)',
    isBuiltIn: true,
  },
];

// ============================================================================
// State
// ============================================================================

let customTypes = $state<BlockType[]>([]);

// ============================================================================
// Derived
// ============================================================================

const allTypes = $derived([...BUILT_IN_TYPES, ...customTypes]);

// ============================================================================
// Actions
// ============================================================================

function addCustomType(label: string, shortLabel: string, color: string): string {
  const id = `custom-${Date.now()}`;
  const newType: BlockType = {
    id,
    label,
    shortLabel: shortLabel.toUpperCase().slice(0, 4),
    color,
    isBuiltIn: false,
  };
  customTypes = [...customTypes, newType];
  saveToLocalStorage();
  return id;
}

function updateCustomType(id: string, updates: Partial<Omit<BlockType, 'id' | 'isBuiltIn'>>): void {
  customTypes = customTypes.map(t =>
    t.id === id ? { ...t, ...updates } : t
  );
  saveToLocalStorage();
}

function deleteCustomType(id: string): void {
  customTypes = customTypes.filter(t => t.id !== id);
  saveToLocalStorage();
}

function getTypeById(id: string): BlockType | undefined {
  return allTypes.find(t => t.id === id);
}

function getTypeColor(id: string): string {
  const type = getTypeById(id);
  return type?.color ?? 'var(--text-muted)';
}

function getTypeShortLabel(id: string): string {
  const type = getTypeById(id);
  return type?.shortLabel ?? id.slice(0, 3).toUpperCase();
}

// ============================================================================
// Persistence
// ============================================================================

function saveToLocalStorage(): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem('aperture-custom-block-types', JSON.stringify(customTypes));
}

function loadFromLocalStorage(): void {
  if (typeof localStorage === 'undefined') return;
  try {
    const stored = localStorage.getItem('aperture-custom-block-types');
    if (stored) {
      customTypes = JSON.parse(stored);
    }
  } catch (e) {
    console.error('Failed to load custom block types:', e);
  }
}

function init(): void {
  loadFromLocalStorage();
}

// ============================================================================
// Export Store Interface
// ============================================================================

export const blockTypesStore = {
  // Reactive getters
  get builtInTypes() { return BUILT_IN_TYPES; },
  get customTypes() { return customTypes; },
  get allTypes() { return allTypes; },

  // Actions
  init,
  addCustomType,
  updateCustomType,
  deleteCustomType,
  getTypeById,
  getTypeColor,
  getTypeShortLabel,
};
