import type {
  FilterDescriptor,
  FilterOperator,
  MutationPlan,
  OverviewPreferences,
  OverviewUiState,
  QuerySummaryChip,
  ResourceOverview,
  ServerCapabilities
} from './types';

export const FILTER_OPERATOR_LABELS: Record<FilterOperator, string> = {
  eq: 'equals',
  ne: 'does not equal',
  lt: 'less than',
  lte: 'less than or equal',
  gt: 'greater than',
  gte: 'greater than or equal',
  in: 'is one of',
  contains: 'contains',
  startsWith: 'starts with',
  endsWith: 'ends with',
  isNull: 'is null',
  isNotNull: 'is not null'
};

export const DEFAULT_PREFERENCES: OverviewPreferences = {
  columnVisibility: {},
  lastInspectorTab: 'request',
  mobileSurface: 'explorer',
  schemaMobileSurface: 'graph'
};

export function buildQuerySummaryChips({
  filters,
  sorting,
  embeds
}: {
  filters: FilterDescriptor[];
  sorting: Array<{ id: string; desc: boolean }>;
  embeds: string[];
}): QuerySummaryChip[] {
  return [
    ...filters.map((filter) => ({
      id: filter.id,
      kind: 'filter' as const,
      label: filter.field,
      value:
        filter.operator === 'isNull' || filter.operator === 'isNotNull'
          ? FILTER_OPERATOR_LABELS[filter.operator]
          : `${FILTER_OPERATOR_LABELS[filter.operator]} ${filter.value}`,
      removeLabel: `Remove filter on ${filter.field}`
    })),
    ...sorting.map((sort) => ({
      id: `sort:${sort.id}`,
      kind: 'sort' as const,
      label: sort.id,
      value: sort.desc ? 'descending' : 'ascending',
      removeLabel: `Remove sort on ${sort.id}`
    })),
    ...embeds.map((embed) => ({
      id: `embed:${embed}`,
      kind: 'embed' as const,
      label: embed,
      value: 'embedded',
      removeLabel: `Remove embed on ${embed}`
    }))
  ];
}

export function getVisibleMutationActions(
  resource: ResourceOverview | null,
  capabilities: ServerCapabilities | null,
  selectedRow: Record<string, unknown> | null
) {
  const canWrite = Boolean(capabilities?.resource_write);
  return {
    createRow: Boolean(canWrite && resource?.kind === 'table' && resource.mutation_capabilities.create_item),
    editRow: Boolean(
      canWrite &&
        selectedRow &&
        resource?.kind === 'table' &&
        resource.mutation_capabilities.update_item
    ),
    deleteRow: Boolean(
      canWrite &&
        selectedRow &&
        resource?.kind === 'table' &&
        resource.mutation_capabilities.delete_item
    ),
    editObject: Boolean(
      canWrite &&
        resource?.kind === 'object' &&
        (resource.mutation_capabilities.patch_object || resource.mutation_capabilities.replace_object)
    )
  };
}

export function buildMutationPlan({
  resource,
  mode,
  originalValue,
  draftText,
  replaceFullItem
}: {
  resource: ResourceOverview;
  mode: 'create' | 'edit' | 'delete' | 'editObject';
  originalValue: unknown;
  draftText: string;
  replaceFullItem: boolean;
}): MutationPlan {
  if (mode === 'delete') {
    const itemPath = buildItemPath(resource, originalValue);
    return {
      method: 'DELETE',
      path: itemPath,
      body: null,
      changedKeys: [],
      requiresConfirmation: true
    };
  }

  const parsedDraft = draftText.trim() ? (JSON.parse(draftText) as unknown) : {};

  if (mode === 'create') {
    return {
      method: 'POST',
      path: `/${encodeURIComponent(resource.name)}`,
      body: JSON.stringify(parsedDraft),
      changedKeys: parsedDraft && typeof parsedDraft === 'object' ? Object.keys(parsedDraft as object) : [],
      requiresConfirmation: false
    };
  }

  if (resource.kind === 'object') {
    if (replaceFullItem) {
      return {
        method: 'PUT',
        path: `/${encodeURIComponent(resource.name)}`,
        body: JSON.stringify(parsedDraft),
        changedKeys: collectChangedKeys(originalValue, parsedDraft),
        requiresConfirmation: true
      };
    }
    const patch = buildPatchPayload(originalValue, parsedDraft);
    return {
      method: 'PATCH',
      path: `/${encodeURIComponent(resource.name)}`,
      body: JSON.stringify(patch),
      changedKeys: Object.keys(patch),
      requiresConfirmation: false
    };
  }

  const itemPath = buildItemPath(resource, originalValue);
  if (replaceFullItem) {
    return {
      method: 'PUT',
      path: itemPath,
      body: JSON.stringify(parsedDraft),
      changedKeys: collectChangedKeys(originalValue, parsedDraft),
      requiresConfirmation: true
    };
  }

  const patch = buildPatchPayload(originalValue, parsedDraft);
  return {
    method: 'PATCH',
    path: itemPath,
    body: JSON.stringify(patch),
    changedKeys: Object.keys(patch),
    requiresConfirmation: false
  };
}

export function buildPatchPayload(originalValue: unknown, nextValue: unknown): Record<string, unknown> {
  if (!isPlainRecord(nextValue)) {
    return {};
  }

  const originalRecord = isPlainRecord(originalValue) ? originalValue : {};
  const patch: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(nextValue)) {
    if (!Object.is(originalRecord[key], value)) {
      patch[key] = value;
    }
  }

  return patch;
}

export function collectChangedKeys(originalValue: unknown, nextValue: unknown): string[] {
  if (!isPlainRecord(nextValue)) {
    return [];
  }

  const originalRecord = isPlainRecord(originalValue) ? originalValue : {};
  const keys = new Set([...Object.keys(originalRecord), ...Object.keys(nextValue)]);
  return [...keys].filter((key) => !Object.is(originalRecord[key], nextValue[key]));
}

export function loadOverviewPreferences(storage: Storage | Pick<Storage, 'getItem'>): OverviewPreferences {
  try {
    const raw = storage.getItem('overview-preferences');
    if (!raw) {
      return DEFAULT_PREFERENCES;
    }
    const parsed = JSON.parse(raw) as Partial<OverviewPreferences>;
    return {
      columnVisibility: parsed.columnVisibility ?? {},
      lastInspectorTab:
        parsed.lastInspectorTab === 'selection' ? 'selection' : DEFAULT_PREFERENCES.lastInspectorTab,
      mobileSurface: parsed.mobileSurface ?? DEFAULT_PREFERENCES.mobileSurface,
      schemaMobileSurface: parsed.schemaMobileSurface ?? DEFAULT_PREFERENCES.schemaMobileSurface
    };
  } catch {
    return DEFAULT_PREFERENCES;
  }
}

export function saveOverviewPreferences(
  storage: Pick<Storage, 'setItem'>,
  preferences: OverviewPreferences
) {
  storage.setItem('overview-preferences', JSON.stringify(preferences));
}

export function createUiState(selectedResource: string | null, readonly: boolean): OverviewUiState {
  return {
    selectedResource,
    selectedRow: null,
    inspectorTab: DEFAULT_PREFERENCES.lastInspectorTab,
    mutationDialog: { open: false, mode: null },
    readonly
  };
}

export function summarizeSchemaDiff(currentText: string, nextText: string): string[] {
  try {
    const current = JSON.parse(currentText) as Record<string, unknown>;
    const next = JSON.parse(nextText) as Record<string, unknown>;
    const currentTables = getNestedKeys(current.tables);
    const nextTables = getNestedKeys(next.tables);
    const all = new Set([...Object.keys(currentTables), ...Object.keys(nextTables)]);
    return [...all]
      .filter((key) => currentTables[key] !== nextTables[key])
      .map((key) => `Changed schema entry: ${key}`);
  } catch {
    return ['JSON changed'];
  }
}

function getNestedKeys(value: unknown): Record<string, string> {
  if (!isPlainRecord(value)) {
    return {};
  }

  const entries: Record<string, string> = {};
  for (const [key, nested] of Object.entries(value)) {
    entries[key] = JSON.stringify(nested);
  }
  return entries;
}

function buildItemPath(resource: ResourceOverview, originalValue: unknown) {
  if (!isPlainRecord(originalValue) || !resource.primary_key) {
    throw new Error(`Cannot build item route for ${resource.name}`);
  }
  const rawId = originalValue[resource.primary_key];
  if (rawId === null || rawId === undefined) {
    throw new Error(`Missing primary key ${resource.primary_key}`);
  }
  return `/${encodeURIComponent(resource.name)}/${encodeURIComponent(String(rawId))}`;
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
