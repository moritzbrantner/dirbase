import { columnTypesAreCompatible, isBoundedColumnType, isNumericLikeColumnType, isStringBackedColumnType } from './columnTypes';
import { mergeSchemaEditorPayload } from './merge';
import { normalizeDeclaredSchema, normalizeDeclaredTable, normalizeSchemaTable } from './normalize';
import type {
  DeclaredSchemaResponse,
  SchemaColumn,
  SchemaResponse
} from '../types';

export function validateSchemaDraft(
  inferredInput: SchemaResponse,
  declaredInput: DeclaredSchemaResponse | null
): string | null {
  const effective = mergeSchemaEditorPayload(inferredInput, declaredInput);
  const declared = normalizeDeclaredSchema(declaredInput);

  for (const [tableName, tableValue] of Object.entries(effective.tables ?? {})) {
    const table = normalizeSchemaTable(tableValue);
    if (table.primary_key && !table.columns?.[table.primary_key]) {
      return `table '${tableName}' declares primary key '${table.primary_key}' but no such column exists`;
    }
    for (const [columnName, column] of Object.entries(table.columns ?? {})) {
      const columnError = validateSchemaColumnConstraints(tableName, columnName, column);
      if (columnError) {
        return columnError;
      }
    }

    for (const [sourceColumn, foreignKeyValue] of Object.entries(table.foreign_keys ?? {})) {
      const sourceColumnSchema = table.columns?.[sourceColumn];
      if (!sourceColumnSchema) {
        return `table '${tableName}' declares foreign key '${sourceColumn}' but no such column exists`;
      }

      const targetTable = effective.tables?.[foreignKeyValue.target_table];
      if (!targetTable) {
        return `table '${tableName}' foreign key '${sourceColumn}' targets unknown table '${foreignKeyValue.target_table}'`;
      }

      const targetColumn = normalizeSchemaTable(targetTable).columns?.[foreignKeyValue.target_column];
      if (!targetColumn) {
        return `table '${tableName}' foreign key '${sourceColumn}' targets unknown column '${foreignKeyValue.target_table}.${foreignKeyValue.target_column}'`;
      }

      if (
        !columnTypesAreCompatible(
          sourceColumnSchema.column_type ?? 'string',
          targetColumn.column_type ?? 'string'
        )
      ) {
        return `table '${tableName}' foreign key '${sourceColumn}' is incompatible with '${foreignKeyValue.target_table}.${foreignKeyValue.target_column}'`;
      }
    }

    const declaredTable = normalizeDeclaredTable(declared.tables?.[tableName]);
    const seenUnique = new Set<string>();
    for (const unique of declaredTable.unique ?? []) {
      if (unique.length === 0) {
        return `table '${tableName}' declares an empty unique constraint`;
      }
      const seenColumns = new Set<string>();
      for (const columnName of unique) {
        if (seenColumns.has(columnName)) {
          return `table '${tableName}' unique constraint contains duplicate column '${columnName}'`;
        }
        seenColumns.add(columnName);
        if (!table.columns?.[columnName]) {
          return `table '${tableName}' unique constraint references unknown column '${columnName}'`;
        }
      }
      const key = [...unique].sort().join('\u001f');
      if (seenUnique.has(key)) {
        return `table '${tableName}' declares duplicate unique constraints`;
      }
      seenUnique.add(key);
    }
  }

  return null;
}

function validateSchemaColumnConstraints(
  tableName: string,
  columnName: string,
  column: SchemaColumn
): string | null {
  const columnType = column.column_type ?? 'string';
  if (column.enum_values !== undefined) {
    if (columnType !== 'string') {
      return `table '${tableName}' column '${columnName}' declares enum_values on non-string type`;
    }
    if (column.enum_values.length === 0) {
      return `table '${tableName}' column '${columnName}' declares empty enum_values`;
    }
    if (new Set(column.enum_values).size !== column.enum_values.length) {
      return `table '${tableName}' column '${columnName}' declares duplicate enum value`;
    }
  }
  if ((column.min !== undefined || column.max !== undefined) && !isBoundedColumnType(columnType)) {
    return `table '${tableName}' column '${columnName}' declares min/max on unsupported type '${columnType}'`;
  }
  const minBound = comparableBoundValue(columnType, column.min);
  const maxBound = comparableBoundValue(columnType, column.max);
  if (column.min !== undefined && minBound === null) {
    return `table '${tableName}' column '${columnName}' declares invalid min`;
  }
  if (column.max !== undefined && maxBound === null) {
    return `table '${tableName}' column '${columnName}' declares invalid max`;
  }
  if (minBound !== undefined && minBound !== null && maxBound !== undefined && maxBound !== null && minBound > maxBound) {
    return `table '${tableName}' column '${columnName}' declares min greater than max`;
  }
  if (
    (column.min_length !== undefined || column.max_length !== undefined) &&
    !isStringBackedColumnType(columnType)
  ) {
    return `table '${tableName}' column '${columnName}' declares length constraints on unsupported type '${columnType}'`;
  }
  if (
    column.min_length !== undefined &&
    column.max_length !== undefined &&
    column.min_length > column.max_length
  ) {
    return `table '${tableName}' column '${columnName}' declares min_length greater than max_length`;
  }
  if (column.pattern !== undefined) {
    if (!isStringBackedColumnType(columnType)) {
      return `table '${tableName}' column '${columnName}' declares pattern on unsupported type '${columnType}'`;
    }
    try {
      new RegExp(column.pattern);
    } catch {
      return `table '${tableName}' column '${columnName}' declares invalid pattern`;
    }
  }
  return null;
}

function comparableBoundValue(columnType: string, value: number | string | undefined): number | null | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (isNumericLikeColumnType(columnType)) {
    return typeof value === 'number' ? value : null;
  }
  if (columnType === 'date' && typeof value === 'string') {
    const timestamp = Date.parse(`${value}T00:00:00Z`);
    return Number.isNaN(timestamp) ? null : timestamp;
  }
  if (columnType === 'datetime' && typeof value === 'string') {
    const timestamp = Date.parse(value);
    return Number.isNaN(timestamp) ? null : timestamp;
  }
  return null;
}
