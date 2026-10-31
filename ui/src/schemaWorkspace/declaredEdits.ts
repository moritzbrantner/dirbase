import type { Connection } from '@xyflow/react';

import { parseSchemaConnection } from '../schemaEditor';
import {
  cleanDeclaredSchema,
  cleanDeclaredTable,
  cleanSchemaColumn,
  cloneDeclaredSchema,
  isDeclaredTableEmpty,
  normalizeDeclaredTable,
  normalizeSchemaColumn,
  normalizeSchemaTable
} from './normalize';
import type {
  DeclaredSchemaResponse,
  DeclaredSchemaTable,
  SchemaColumn,
  SchemaColumnOverrideInput,
  SchemaResponse
} from '../types';

export function setDeclaredTableKind(
  declaredInput: DeclaredSchemaResponse,
  tableName: string,
  kind: string | null
): DeclaredSchemaResponse {
  return updateDeclaredTable(declaredInput, tableName, (table) => {
    table.kind = kind || undefined;
  });
}

export function setDeclaredPrimaryKey(
  declaredInput: DeclaredSchemaResponse,
  tableName: string,
  primaryKey: string | null
): DeclaredSchemaResponse {
  return updateDeclaredTable(declaredInput, tableName, (table) => {
    table.primary_key = primaryKey || undefined;
  });
}

export function setDeclaredUniqueConstraints(
  declaredInput: DeclaredSchemaResponse,
  tableName: string,
  unique: string[][]
): DeclaredSchemaResponse {
  return updateDeclaredTable(declaredInput, tableName, (table) => {
    table.unique = unique.length > 0 ? unique : undefined;
  });
}

export function setDeclaredColumnOverride(
  declaredInput: DeclaredSchemaResponse,
  effectiveInput: SchemaResponse,
  tableName: string,
  columnName: string,
  next: SchemaColumnOverrideInput
): DeclaredSchemaResponse {
  const effectiveTable = normalizeSchemaTable(effectiveInput.tables?.[tableName]);
  const effectiveColumn = normalizeSchemaColumn(effectiveTable.columns?.[columnName]);
  const inferredType = effectiveColumn.column_type ?? 'string';
  const inferredNullable = effectiveColumn.nullable ?? true;

  return updateDeclaredTable(declaredInput, tableName, (table) => {
    const columns = { ...(table.columns ?? {}) };
    const existing = cleanSchemaColumn(columns[columnName]);
    const nextColumnType = next.columnType ?? inferredType;
    const nextNullable = next.nullable ?? inferredNullable;
    const nextColumn: SchemaColumn = {
      ...existing,
      column_type: nextColumnType,
      nullable: nextNullable
    };

    if ('enumValues' in next) {
      if (!Array.isArray(next.enumValues) || next.enumValues.length === 0) {
        delete nextColumn.enum_values;
      } else {
        nextColumn.enum_values = next.enumValues;
      }
    }
    if ('min' in next) {
      if (typeof next.min !== 'number' && typeof next.min !== 'string') delete nextColumn.min;
      else nextColumn.min = next.min;
    }
    if ('max' in next) {
      if (typeof next.max !== 'number' && typeof next.max !== 'string') delete nextColumn.max;
      else nextColumn.max = next.max;
    }
    if ('minLength' in next) {
      if (typeof next.minLength !== 'number') delete nextColumn.min_length;
      else nextColumn.min_length = next.minLength;
    }
    if ('maxLength' in next) {
      if (typeof next.maxLength !== 'number') delete nextColumn.max_length;
      else nextColumn.max_length = next.maxLength;
    }
    if ('pattern' in next) {
      if (next.pattern === null || next.pattern === '') delete nextColumn.pattern;
      else nextColumn.pattern = next.pattern;
    }

    if (next.columnType === null && next.nullable === null && onlyTypeAndNullability(next)) {
      delete columns[columnName];
      table.columns = columns;
      return;
    }

    if (
      nextColumnType === inferredType &&
      nextNullable === inferredNullable &&
      !hasColumnConstraintOverrides(nextColumn)
    ) {
      delete columns[columnName];
      table.columns = columns;
      return;
    }

    columns[columnName] = nextColumn;
    table.columns = columns;
  });
}

export function upsertDeclaredRelationship(
  declaredInput: DeclaredSchemaResponse,
  connection: { sourceTable: string; sourceColumn: string; targetTable: string; targetColumn: string }
): DeclaredSchemaResponse {
  return updateDeclaredTable(declaredInput, connection.sourceTable, (table) => {
    table.foreign_keys = {
      ...(table.foreign_keys ?? {}),
      [connection.sourceColumn]: {
        target_table: connection.targetTable,
        target_column: connection.targetColumn
      }
    };
    table.suppressed_foreign_keys = (table.suppressed_foreign_keys ?? []).filter(
      (columnName) => columnName !== connection.sourceColumn
    );
  });
}

export function stageRelationshipFromConnection(
  declaredInput: DeclaredSchemaResponse,
  connection: Connection
): { declared: DeclaredSchemaResponse; selection: { tableName: string; relationSourceColumn: string } } | null {
  const parsedConnection = parseSchemaConnection(connection);
  if (!parsedConnection) {
    return null;
  }

  return {
    declared: upsertDeclaredRelationship(declaredInput, parsedConnection),
    selection: {
      tableName: parsedConnection.sourceTable,
      relationSourceColumn: parsedConnection.sourceColumn
    }
  };
}

export function removeDeclaredRelationship(
  declaredInput: DeclaredSchemaResponse,
  inferredInput: SchemaResponse,
  tableName: string,
  sourceColumn: string
): DeclaredSchemaResponse {
  const inferredForeignKey = normalizeSchemaTable(inferredInput.tables?.[tableName]).foreign_keys?.[
    sourceColumn
  ];

  return updateDeclaredTable(declaredInput, tableName, (table) => {
    const foreignKeys = { ...(table.foreign_keys ?? {}) };
    delete foreignKeys[sourceColumn];
    table.foreign_keys = foreignKeys;

    if (inferredForeignKey) {
      table.suppressed_foreign_keys = [
        ...new Set([...(table.suppressed_foreign_keys ?? []), sourceColumn])
      ];
    } else {
      table.suppressed_foreign_keys = (table.suppressed_foreign_keys ?? []).filter(
        (columnName) => columnName !== sourceColumn
      );
    }
  });
}

export function resetDeclaredRelationship(
  declaredInput: DeclaredSchemaResponse,
  tableName: string,
  sourceColumn: string
): DeclaredSchemaResponse {
  return updateDeclaredTable(declaredInput, tableName, (table) => {
    const foreignKeys = { ...(table.foreign_keys ?? {}) };
    delete foreignKeys[sourceColumn];
    table.foreign_keys = foreignKeys;
    table.suppressed_foreign_keys = (table.suppressed_foreign_keys ?? []).filter(
      (columnName) => columnName !== sourceColumn
    );
  });
}

function updateDeclaredTable(
  declaredInput: DeclaredSchemaResponse,
  tableName: string,
  mutate: (table: DeclaredSchemaTable) => void
): DeclaredSchemaResponse {
  const declared = cloneDeclaredSchema(declaredInput);
  const tables = { ...(declared.tables ?? {}) };
  const table = normalizeDeclaredTable(tables[tableName]);
  mutate(table);
  const nextTable = cleanDeclaredTable(table);

  if (isDeclaredTableEmpty(nextTable)) {
    delete tables[tableName];
  } else {
    tables[tableName] = nextTable;
  }

  return cleanDeclaredSchema({
    ...declared,
    tables
  });
}

function onlyTypeAndNullability(next: SchemaColumnOverrideInput): boolean {
  return !(
    'enumValues' in next ||
    'min' in next ||
    'max' in next ||
    'minLength' in next ||
    'maxLength' in next ||
    'pattern' in next
  );
}

function hasColumnConstraintOverrides(column: SchemaColumn): boolean {
  return Boolean(
    column.enum_values?.length ||
      column.min !== undefined ||
      column.max !== undefined ||
      column.min_length !== undefined ||
      column.max_length !== undefined ||
      column.pattern
  );
}
