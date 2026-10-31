import { isRecord } from '../helpers';
import { COLUMN_TYPES } from './columnTypes';
import type {
  DeclaredSchemaResponse,
  DeclaredSchemaTable,
  SchemaColumn,
  SchemaForeignKey,
  SchemaManyToManyRelation,
  SchemaResponse,
  SchemaTable
} from '../types';

const EMPTY_DECLARED_SCHEMA: DeclaredSchemaResponse = { tables: {} };
const TABLE_KINDS = new Set(['object', 'relation', 'unknown']);

export function normalizeSchema(schema: SchemaResponse | null | undefined): SchemaResponse {
  if (!schema || !isRecord(schema.tables)) {
    return { tables: {} };
  }

  return {
    ...schema,
    tables: Object.fromEntries(
      Object.entries(schema.tables).map(([tableName, table]) => [
        tableName,
        normalizeSchemaTable(table)
      ])
    )
  };
}

export function normalizeDeclaredSchema(
  schema: DeclaredSchemaResponse | null | undefined
): DeclaredSchemaResponse {
  if (!schema || !isRecord(schema.tables)) {
    return cloneDeclaredSchema(EMPTY_DECLARED_SCHEMA);
  }

  return cleanDeclaredSchema({
    ...schema,
    tables: Object.fromEntries(
      Object.entries(schema.tables).map(([tableName, table]) => [
        tableName,
        normalizeDeclaredTable(table)
      ])
    )
  });
}

export function cloneDeclaredSchema(schema: DeclaredSchemaResponse): DeclaredSchemaResponse {
  return JSON.parse(JSON.stringify(normalizeDeclaredSchema(schema))) as DeclaredSchemaResponse;
}

export function formatDeclaredSchema(schema: DeclaredSchemaResponse): string {
  return `${JSON.stringify(normalizeDeclaredSchema(schema), null, 2)}\n`;
}

export function cleanDeclaredSchema(schema: DeclaredSchemaResponse): DeclaredSchemaResponse {
  const tables: Record<string, DeclaredSchemaTable> = {};
  for (const [tableName, tableValue] of Object.entries(schema.tables ?? {})) {
    const table = cleanDeclaredTable(normalizeDeclaredTable(tableValue));
    if (!isDeclaredTableEmpty(table)) {
      tables[tableName] = table;
    }
  }

  return {
    ...schema,
    tables
  };
}

export function cleanDeclaredTable(table: DeclaredSchemaTable): DeclaredSchemaTable {
  const columns: Record<string, SchemaColumn> = {};
  for (const [columnName, columnValue] of Object.entries(table.columns ?? {})) {
    const column = cleanSchemaColumn(columnValue);
    if (column.column_type !== undefined && column.nullable !== undefined) {
      columns[columnName] = column;
    }
  }

  const foreignKeys: Record<string, SchemaForeignKey> = {};
  for (const [columnName, foreignKeyValue] of Object.entries(table.foreign_keys ?? {})) {
    if (isValidForeignKey(foreignKeyValue)) {
      foreignKeys[columnName] = {
        target_table: foreignKeyValue.target_table,
        target_column: foreignKeyValue.target_column
      };
    }
  }
  const suppressed_foreign_keys = [...new Set((table.suppressed_foreign_keys ?? []).filter(Boolean))].sort();
  const unique = cleanUniqueConstraints(table.unique);

  return {
    ...table,
    kind: normalizeTableKind(table.kind),
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : undefined,
    columns: Object.keys(columns).length > 0 ? columns : undefined,
    foreign_keys: Object.keys(foreignKeys).length > 0 ? foreignKeys : undefined,
    suppressed_foreign_keys:
      suppressed_foreign_keys.length > 0 ? suppressed_foreign_keys : undefined,
    unique: unique.length > 0 ? unique : undefined
  };
}

export function isDeclaredTableEmpty(table: DeclaredSchemaTable): boolean {
  return (
    !table.kind &&
    !table.primary_key &&
    !table.columns &&
    !table.foreign_keys &&
    !table.suppressed_foreign_keys &&
    !table.unique
  );
}

export function cleanSchemaTable(table: SchemaTable): SchemaTable {
  const columns: Record<string, SchemaColumn> = {};
  for (const [columnName, columnValue] of Object.entries(table.columns ?? {})) {
    columns[columnName] = cleanSchemaColumn(columnValue);
  }

  const foreignKeys: Record<string, SchemaForeignKey> = {};
  for (const [columnName, foreignKeyValue] of Object.entries(table.foreign_keys ?? {})) {
    if (isValidForeignKey(foreignKeyValue)) {
      foreignKeys[columnName] = {
        target_table: foreignKeyValue.target_table,
        target_column: foreignKeyValue.target_column
      };
    }
  }

  const manyToMany: Record<string, SchemaManyToManyRelation> = {};
  for (const [relationName, relationValue] of Object.entries(table.many_to_many ?? {})) {
    if (isValidManyToManyRelation(relationValue)) {
      manyToMany[relationName] = {
        through_table: relationValue.through_table,
        source_column: relationValue.source_column,
        source_target_column: relationValue.source_target_column,
        target_table: relationValue.target_table,
        target_column: relationValue.target_column,
        through_target_column: relationValue.through_target_column
      };
    }
  }
  const unique = cleanUniqueConstraints(table.unique);

  return {
    ...table,
    kind: normalizeTableKind(table.kind) ?? 'unknown',
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : null,
    columns,
    foreign_keys: foreignKeys,
    many_to_many: manyToMany
  };
}

export function normalizeDeclaredTable(table: unknown): DeclaredSchemaTable {
  if (!isRecord(table)) {
    return {};
  }

  const columns: Record<string, SchemaColumn> = {};
  if (isRecord(table.columns)) {
    for (const [columnName, columnValue] of Object.entries(table.columns)) {
      columns[columnName] = cleanSchemaColumn(columnValue);
    }
  }

  const foreignKeys: Record<string, SchemaForeignKey> = {};
  if (isRecord(table.foreign_keys)) {
    for (const [columnName, foreignKeyValue] of Object.entries(table.foreign_keys)) {
      if (isValidForeignKey(foreignKeyValue)) {
        foreignKeys[columnName] = {
          target_table: foreignKeyValue.target_table,
          target_column: foreignKeyValue.target_column
        };
      }
    }
  }
  const unique = cleanUniqueConstraints(table.unique);

  return {
    ...table,
    kind: normalizeTableKind(table.kind),
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : undefined,
    columns: Object.keys(columns).length > 0 ? columns : undefined,
    foreign_keys: Object.keys(foreignKeys).length > 0 ? foreignKeys : undefined,
    suppressed_foreign_keys: Array.isArray(table.suppressed_foreign_keys)
      ? table.suppressed_foreign_keys.filter((value): value is string => typeof value === 'string')
      : undefined,
    unique: unique.length > 0 ? unique : undefined
  };
}

export function normalizeSchemaTable(table: unknown): SchemaTable {
  if (!isRecord(table)) {
    return { kind: 'unknown', primary_key: null, columns: {}, foreign_keys: {}, many_to_many: {} };
  }

  const columns: Record<string, SchemaColumn> = {};
  if (isRecord(table.columns)) {
    for (const [columnName, columnValue] of Object.entries(table.columns)) {
      columns[columnName] = cleanSchemaColumn(columnValue);
    }
  }

  const foreignKeys: Record<string, SchemaForeignKey> = {};
  if (isRecord(table.foreign_keys)) {
    for (const [columnName, foreignKeyValue] of Object.entries(table.foreign_keys)) {
      if (isValidForeignKey(foreignKeyValue)) {
        foreignKeys[columnName] = {
          target_table: foreignKeyValue.target_table,
          target_column: foreignKeyValue.target_column
        };
      }
    }
  }

  const manyToMany: Record<string, SchemaManyToManyRelation> = {};
  if (isRecord(table.many_to_many)) {
    for (const [relationName, relationValue] of Object.entries(table.many_to_many)) {
      if (isValidManyToManyRelation(relationValue)) {
        manyToMany[relationName] = {
          through_table: relationValue.through_table,
          source_column: relationValue.source_column,
          source_target_column: relationValue.source_target_column,
          target_table: relationValue.target_table,
          target_column: relationValue.target_column,
          through_target_column: relationValue.through_target_column
        };
      }
    }
  }

  return cleanSchemaTable({
    ...table,
    kind: normalizeTableKind(table.kind) ?? 'unknown',
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : null,
    columns,
    foreign_keys: foreignKeys,
    many_to_many: manyToMany
  });
}

export function cleanSchemaColumn(column: unknown): SchemaColumn {
  if (!isRecord(column)) {
    return {};
  }

  const enumValues = Array.isArray(column.enum_values)
    ? column.enum_values.filter((value): value is string => typeof value === 'string')
    : undefined;
  return {
    ...column,
    column_type: normalizeColumnType(column.column_type) ?? undefined,
    nullable: typeof column.nullable === 'boolean' ? column.nullable : undefined,
    enum_values: enumValues && enumValues.length > 0 ? enumValues : undefined,
    min: cleanBoundValue(column.min),
    max: cleanBoundValue(column.max),
    min_length:
      typeof column.min_length === 'number' && Number.isInteger(column.min_length) && column.min_length >= 0
        ? column.min_length
        : undefined,
    max_length:
      typeof column.max_length === 'number' && Number.isInteger(column.max_length) && column.max_length >= 0
        ? column.max_length
        : undefined,
    pattern: typeof column.pattern === 'string' && column.pattern ? column.pattern : undefined
  };
}

function cleanBoundValue(value: unknown): number | string | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === 'string' && value) {
    return value;
  }
  return undefined;
}

function cleanUniqueConstraints(value: unknown): string[][] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .flatMap((constraint) => {
      if (!Array.isArray(constraint)) {
        return [];
      }
      const columns = [
        ...new Set(
          constraint
            .filter((column): column is string => typeof column === 'string' && Boolean(column.trim()))
            .map((column) => column.trim())
        )
      ];
      return columns.length > 0 ? [columns] : [];
    });
}

export function normalizeSchemaColumn(column: unknown): SchemaColumn {
  return cleanSchemaColumn(column);
}

function normalizeColumnType(value: unknown): string | null {
  return typeof value === 'string' && COLUMN_TYPES.has(value) ? value : null;
}

function normalizeTableKind(value: unknown): string | null {
  return typeof value === 'string' && TABLE_KINDS.has(value) ? value : null;
}

function isValidForeignKey(value: unknown): value is SchemaForeignKey {
  return (
    isRecord(value) &&
    typeof value.target_table === 'string' &&
    Boolean(value.target_table.trim()) &&
    typeof value.target_column === 'string' &&
    Boolean(value.target_column.trim())
  );
}

function isValidManyToManyRelation(value: unknown): value is SchemaManyToManyRelation {
  return (
    isRecord(value) &&
    typeof value.through_table === 'string' &&
    Boolean(value.through_table.trim()) &&
    typeof value.source_column === 'string' &&
    Boolean(value.source_column.trim()) &&
    typeof value.source_target_column === 'string' &&
    Boolean(value.source_target_column.trim()) &&
    typeof value.target_table === 'string' &&
    Boolean(value.target_table.trim()) &&
    typeof value.target_column === 'string' &&
    Boolean(value.target_column.trim()) &&
    typeof value.through_target_column === 'string' &&
    Boolean(value.through_target_column.trim())
  );
}
