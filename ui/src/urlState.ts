import type {
  FilterDescriptor,
  FilterOperator,
  OverviewUrlState,
  OverviewView,
  ResourceOverview,
  SortDescriptor
} from './types';

export const FILTER_OPERATORS: FilterOperator[] = [
  'eq',
  'ne',
  'lt',
  'lte',
  'gt',
  'gte',
  'in',
  'contains',
  'startsWith',
  'endsWith',
  'isNull',
  'isNotNull'
];

const RESERVED_KEYS = new Set(['resource', 'view', 'sort', '_sort', 'page', '_page', 'per_page', '_per_page', 'embed', '_embed']);

export const DEFAULT_PER_PAGE = 25;
export const PAGE_SIZE_OPTIONS = [10, 25, 50, 100];

export function parseOverviewState(search: string): OverviewUrlState {
  const params = new URLSearchParams(search);
  const view = normalizeView(params.get('view'));
  const sorting = parseSorting(params.get('sort') ?? params.get('_sort'));
  const embeds = parseCsv(params.get('embed') ?? params.get('_embed'));
  const filters: FilterDescriptor[] = [];

  for (const [key, value] of params.entries()) {
    if (RESERVED_KEYS.has(key)) {
      continue;
    }
    const [field, operator] = parseFilterKey(key);
    filters.push({
      id: createFilterId(field, operator, filters.length),
      field,
      operator,
      value
    });
  }

  return {
    resource: params.get('resource'),
    view,
    page: parsePositiveInt(params.get('page') ?? params.get('_page'), 1),
    perPage: parsePositiveInt(params.get('per_page') ?? params.get('_per_page'), DEFAULT_PER_PAGE),
    sorting,
    embeds,
    filters
  };
}

export function buildBrowserQueryString(state: OverviewUrlState): string {
  const params = new URLSearchParams();
  if (state.resource) {
    params.set('resource', state.resource);
  }
  if (state.view !== 'explore') {
    params.set('view', state.view);
  }

  const resourceParams = buildResourceSearchParams(state);
  for (const [key, value] of resourceParams.entries()) {
    params.append(key, value);
  }

  const queryString = params.toString();
  return queryString ? `?${queryString}` : '';
}

export function buildResourceSearchParams(state: Pick<OverviewUrlState, 'page' | 'perPage' | 'sorting' | 'embeds' | 'filters'>): URLSearchParams {
  const params = new URLSearchParams();
  params.set('page', String(state.page));
  params.set('per_page', String(state.perPage));

  if (state.sorting.length > 0) {
    params.set(
      'sort',
      state.sorting
        .map((sort) => (sort.desc ? `-${sort.id}` : sort.id))
        .join(',')
    );
  }

  if (state.embeds.length > 0) {
    params.set('embed', state.embeds.join(','));
  }

  for (const filter of state.filters) {
    const requiresValue = filter.operator !== 'isNull' && filter.operator !== 'isNotNull';
    if (!filter.field.trim() || (requiresValue && !filter.value.trim())) {
      continue;
    }
    const key = filter.operator === 'eq' ? filter.field : `${filter.field}:${filter.operator}`;
    params.append(key, requiresValue ? filter.value : 'true');
  }

  return params;
}

export function buildResourceRequestPath(resource: ResourceOverview | null | undefined, state: OverviewUrlState): string {
  if (!resource) {
    return '/';
  }
  if (resource.kind !== 'table') {
    return `/${encodeURIComponent(resource.name)}`;
  }
  const params = buildResourceSearchParams(state);
  return params.toString()
    ? `/${encodeURIComponent(resource.name)}?${params.toString()}`
    : `/${encodeURIComponent(resource.name)}`;
}

export function resetTableState(resource: string, view: OverviewView = 'explore'): OverviewUrlState {
  return {
    resource,
    view,
    page: 1,
    perPage: DEFAULT_PER_PAGE,
    filters: [],
    sorting: [],
    embeds: []
  };
}

export function nextSorting(current: SortDescriptor[], columnId: string, multiSort: boolean): SortDescriptor[] {
  const previous = multiSort
    ? [...current]
    : current.find((entry) => entry.id === columnId)
      ? [current.find((entry) => entry.id === columnId)!]
      : [];
  const index = previous.findIndex((entry) => entry.id === columnId);
  if (index === -1) {
    previous.push({ id: columnId, desc: false });
    return previous;
  }
  if (!previous[index].desc) {
    previous[index] = { ...previous[index], desc: true };
    return previous;
  }
  previous.splice(index, 1);
  return previous;
}

export function createFilter(field: string): FilterDescriptor {
  return {
    id: createFilterId(field, 'eq', Math.random()),
    field,
    operator: 'eq',
    value: ''
  };
}

function parseSorting(raw: string | null): SortDescriptor[] {
  if (!raw) {
    return [];
  }
  return raw
    .split(',')
    .map((segment) => segment.trim())
    .filter(Boolean)
    .map((segment) =>
      segment.startsWith('-')
        ? { id: segment.slice(1), desc: true }
        : { id: segment, desc: false }
    )
    .filter((entry) => entry.id.length > 0);
}

function parseCsv(raw: string | null): string[] {
  if (!raw) {
    return [];
  }
  return raw.split(',').map((value) => value.trim()).filter(Boolean);
}

function parseFilterKey(key: string): [string, FilterOperator] {
  if (key.includes(':')) {
    const [field, operator] = key.split(':', 2);
    if (field && isFilterOperator(operator)) {
      return [field, operator];
    }
  }

  const underscoreIndex = key.lastIndexOf('_');
  if (underscoreIndex > 0) {
    const field = key.slice(0, underscoreIndex);
    const operator = key.slice(underscoreIndex + 1);
    if (field && isFilterOperator(operator)) {
      return [field, operator];
    }
  }

  return [key, 'eq'];
}

function isFilterOperator(value: string): value is FilterOperator {
  return FILTER_OPERATORS.includes(value as FilterOperator);
}

function normalizeView(value: string | null): OverviewView {
  return value === 'raw' ? 'raw' : 'explore';
}

function parsePositiveInt(value: string | null, fallback: number): number {
  if (!value) {
    return fallback;
  }
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function createFilterId(field: string, operator: FilterOperator, suffix: number): string {
  return `${field}:${operator}:${suffix}`;
}
