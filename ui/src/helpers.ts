import type { OverviewRelation, PaginatedResponse, ResourceOverview } from './types';

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isPaginatedResponse(value: unknown): value is PaginatedResponse {
  return isRecord(value) && Array.isArray(value.data) && typeof value.page === 'number';
}

export function getTableRows(value: unknown): Record<string, unknown>[] {
  const rawRows = isPaginatedResponse(value)
    ? value.data
    : Array.isArray(value)
      ? value
      : [];

  return rawRows.map((row) => (isRecord(row) ? row : { value: row }));
}

export function shouldUseTableView(resource: ResourceOverview | null, rawMode: boolean): boolean {
  return Boolean(resource && resource.kind === 'table' && !rawMode);
}

export function getColumnNames(resource: ResourceOverview, rows: Record<string, unknown>[]): string[] {
  const names: string[] = [];

  for (const column of resource.columns) {
    if (!names.includes(column.name)) {
      names.push(column.name);
    }
  }
  for (const field of resource.field_names) {
    if (!names.includes(field)) {
      names.push(field);
    }
  }
  for (const row of rows.slice(0, 20)) {
    for (const key of Object.keys(row)) {
      if (!names.includes(key)) {
        names.push(key);
      }
    }
  }

  const primaryKey = resource.primary_key;
  if (!primaryKey || !names.includes(primaryKey)) {
    return names;
  }
  return [primaryKey, ...names.filter((name) => name !== primaryKey)];
}

export function formatJson(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function truncate(value: string, maxLength = 96): string {
  return value.length <= maxLength ? value : `${value.slice(0, maxLength - 1)}...`;
}

export function summarizeValue(value: unknown): string {
  if (value === null) {
    return 'null';
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (Array.isArray(value)) {
    return `Array(${value.length})`;
  }
  if (isRecord(value)) {
    const keys = Object.keys(value);
    return `{ ${keys.slice(0, 3).join(', ')}${keys.length > 3 ? ', ...' : ''} }`;
  }
  return String(value);
}

export function coerceRelationValue(row: Record<string, unknown>, relation: OverviewRelation): string | null {
  const raw = row[relation.source_column];
  if (raw === null || raw === undefined) {
    return null;
  }
  if (typeof raw === 'string' || typeof raw === 'number' || typeof raw === 'boolean') {
    return String(raw);
  }
  if (isRecord(raw)) {
    const nested = raw[relation.target_column];
    if (typeof nested === 'string' || typeof nested === 'number' || typeof nested === 'boolean') {
      return String(nested);
    }
  }
  return null;
}

export function coerceIncomingRelationValue(
  row: Record<string, unknown>,
  relation: OverviewRelation
): string | null {
  const raw = row[relation.target_column];
  if (raw === null || raw === undefined) {
    return null;
  }
  if (typeof raw === 'string' || typeof raw === 'number' || typeof raw === 'boolean') {
    return String(raw);
  }
  return null;
}
