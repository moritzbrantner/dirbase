import { isDeclaredTableEmpty, normalizeDeclaredTable, normalizeSchemaTable } from './normalize';
import type {
  DeclaredSchemaResponse,
  SchemaResponse
} from '../types';

export function isSchemaTableManual(
  declared: DeclaredSchemaResponse,
  tableName: string
): boolean {
  const table = normalizeDeclaredTable(declared.tables?.[tableName]);
  return !isDeclaredTableEmpty(table);
}

export function getSchemaTableNames(
  inferred: SchemaResponse,
  declared: DeclaredSchemaResponse,
  discoveredTables: string[]
): string[] {
  const discovered = new Set(discoveredTables);
  return [...new Set([...discoveredTables, ...Object.keys(inferred.tables ?? {}), ...Object.keys(declared.tables ?? {})])]
    .filter((tableName) => discovered.has(tableName))
    .sort((left, right) => left.localeCompare(right));
}

export function isColumnOverridden(
  declared: DeclaredSchemaResponse,
  tableName: string,
  columnName: string
): boolean {
  return Boolean(normalizeDeclaredTable(declared.tables?.[tableName]).columns?.[columnName]);
}

export function getRelationOrigin(
  inferred: SchemaResponse,
  declared: DeclaredSchemaResponse,
  tableName: string,
  sourceColumn: string
): 'manual' | 'suppressed' | 'inferred' | 'none' {
  const declaredTable = normalizeDeclaredTable(declared.tables?.[tableName]);
  if (declaredTable.foreign_keys?.[sourceColumn]) {
    return 'manual';
  }
  if ((declaredTable.suppressed_foreign_keys ?? []).includes(sourceColumn)) {
    return 'suppressed';
  }
  if (normalizeSchemaTable(inferred.tables?.[tableName]).foreign_keys?.[sourceColumn]) {
    return 'inferred';
  }
  return 'none';
}

export function getTableRelationCount(schema: SchemaResponse, tableName: string): number {
  const table = normalizeSchemaTable(schema.tables?.[tableName]);
  return (
    Object.keys(table.foreign_keys ?? {}).length + Object.keys(table.many_to_many ?? {}).length
  );
}
