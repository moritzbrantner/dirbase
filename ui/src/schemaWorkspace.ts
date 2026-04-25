import type { Connection } from '@xyflow/react';

import { isRecord } from './helpers';
import { parseSchemaConnection } from './schemaEditor';
import type {
  DeclaredSchemaResponse,
  DeclaredSchemaTable,
  SchemaColumn,
  SchemaEditorPayload,
  SchemaForeignKey,
  SchemaManyToManyRelation,
  SchemaResponse,
  SchemaTable
} from './types';

const EMPTY_DECLARED_SCHEMA: DeclaredSchemaResponse = { tables: {} };
const COLUMN_TYPES = new Set(['integer', 'float', 'boolean', 'string', 'json']);
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

export function mergeSchemaEditorPayload(
  inferredInput: SchemaResponse,
  declaredInput: DeclaredSchemaResponse | null
): SchemaResponse {
  const inferred = normalizeSchema(inferredInput);
  const declared = normalizeDeclaredSchema(declaredInput);
  const tableNames = new Set([
    ...Object.keys(inferred.tables ?? {}),
    ...Object.keys(declared.tables ?? {})
  ]);
  const tables: Record<string, SchemaTable> = {};
  const manualKinds: Record<string, string | null> = {};

  for (const tableName of tableNames) {
    const inferredTable = normalizeSchemaTable(inferred.tables?.[tableName]);
    const declaredTable = normalizeDeclaredTable(declared.tables?.[tableName]);
    const columns = {
      ...inferredTable.columns,
      ...declaredTable.columns
    };
    const suppressed = new Set(declaredTable.suppressed_foreign_keys ?? []);
    const manualKeys = new Set(Object.keys(declaredTable.foreign_keys ?? {}));
    const foreignKeys = Object.fromEntries(
      Object.entries(inferredTable.foreign_keys ?? {}).filter(
        ([columnName]) => !suppressed.has(columnName) && !manualKeys.has(columnName)
      )
    );

    Object.assign(foreignKeys, declaredTable.foreign_keys ?? {});

    const primaryKey =
      declaredTable.primary_key !== undefined ? declaredTable.primary_key : inferredTable.primary_key;
    manualKinds[tableName] = declaredTable.kind ?? null;

    tables[tableName] = cleanSchemaTable({
      ...inferredTable,
      columns,
      primary_key: primaryKey ?? null,
      foreign_keys: foreignKeys,
      kind: inferredTable.kind ?? 'unknown',
      many_to_many: {}
    });
  }

  deriveManyToManyTables(tables);
  for (const [tableName, table] of Object.entries(tables)) {
    table.kind = manualKinds[tableName] ?? inferSchemaKind(tableName, table, tables);
  }

  return { tables };
}

export function validateSchemaDraft(
  inferredInput: SchemaResponse,
  declaredInput: DeclaredSchemaResponse | null
): string | null {
  const effective = mergeSchemaEditorPayload(inferredInput, declaredInput);

  for (const [tableName, tableValue] of Object.entries(effective.tables ?? {})) {
    const table = normalizeSchemaTable(tableValue);
    if (table.primary_key && !table.columns?.[table.primary_key]) {
      return `table '${tableName}' declares primary key '${table.primary_key}' but no such column exists`;
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
  }

  return null;
}

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

export function setDeclaredColumnOverride(
  declaredInput: DeclaredSchemaResponse,
  effectiveInput: SchemaResponse,
  tableName: string,
  columnName: string,
  next: { columnType: string | null; nullable: boolean | null }
): DeclaredSchemaResponse {
  const effectiveTable = normalizeSchemaTable(effectiveInput.tables?.[tableName]);
  const effectiveColumn = normalizeSchemaColumn(effectiveTable.columns?.[columnName]);
  const inferredType = effectiveColumn.column_type ?? 'string';
  const inferredNullable = effectiveColumn.nullable ?? true;

  return updateDeclaredTable(declaredInput, tableName, (table) => {
    const columns = { ...(table.columns ?? {}) };
    const nextColumnType = next.columnType ?? inferredType;
    const nextNullable = next.nullable ?? inferredNullable;

    if (next.columnType === null && next.nullable === null) {
      delete columns[columnName];
      table.columns = columns;
      return;
    }

    if (nextColumnType === inferredType && nextNullable === inferredNullable) {
      delete columns[columnName];
      table.columns = columns;
      return;
    }

    columns[columnName] = {
      column_type: nextColumnType,
      nullable: nextNullable
    };
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

export function getSchemaWorkspaceSnapshot(payload: SchemaEditorPayload | undefined) {
  const inferred = normalizeSchema(payload?.inferred);
  const declared = normalizeDeclaredSchema(payload?.declared);
  const effective = mergeSchemaEditorPayload(inferred, declared);

  return {
    inferred,
    declared,
    effective,
    savePath: payload?.save_path ?? 'schema.json'
  };
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

function cleanDeclaredSchema(schema: DeclaredSchemaResponse): DeclaredSchemaResponse {
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

function cleanDeclaredTable(table: DeclaredSchemaTable): DeclaredSchemaTable {
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

  return {
    ...table,
    kind: normalizeTableKind(table.kind),
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : undefined,
    columns: Object.keys(columns).length > 0 ? columns : undefined,
    foreign_keys: Object.keys(foreignKeys).length > 0 ? foreignKeys : undefined,
    suppressed_foreign_keys:
      suppressed_foreign_keys.length > 0 ? suppressed_foreign_keys : undefined
  };
}

function isDeclaredTableEmpty(table: DeclaredSchemaTable): boolean {
  return !table.kind && !table.primary_key && !table.columns && !table.foreign_keys && !table.suppressed_foreign_keys;
}

function cleanSchemaTable(table: SchemaTable): SchemaTable {
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

function normalizeDeclaredTable(table: unknown): DeclaredSchemaTable {
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

  return {
    ...table,
    kind: normalizeTableKind(table.kind),
    primary_key:
      typeof table.primary_key === 'string' && table.primary_key.trim() ? table.primary_key : undefined,
    columns: Object.keys(columns).length > 0 ? columns : undefined,
    foreign_keys: Object.keys(foreignKeys).length > 0 ? foreignKeys : undefined,
    suppressed_foreign_keys: Array.isArray(table.suppressed_foreign_keys)
      ? table.suppressed_foreign_keys.filter((value): value is string => typeof value === 'string')
      : undefined
  };
}

function normalizeSchemaTable(table: unknown): SchemaTable {
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

function cleanSchemaColumn(column: unknown): SchemaColumn {
  if (!isRecord(column)) {
    return {};
  }

  return {
    ...column,
    column_type: normalizeColumnType(column.column_type) ?? undefined,
    nullable: typeof column.nullable === 'boolean' ? column.nullable : undefined
  };
}

function normalizeSchemaColumn(column: unknown): SchemaColumn {
  return cleanSchemaColumn(column);
}

function normalizeColumnType(value: unknown): string | null {
  return typeof value === 'string' && COLUMN_TYPES.has(value) ? value : null;
}

function normalizeTableKind(value: unknown): string | null {
  return typeof value === 'string' && TABLE_KINDS.has(value) ? value : null;
}

function inferSchemaKind(
  tableName: string,
  table: SchemaTable,
  tables: Record<string, SchemaTable>
): string {
  if (table.primary_key) {
    return 'object';
  }
  if (isStrictJunctionTable(tableName, table, tables)) {
    return 'relation';
  }
  return 'unknown';
}

function columnTypesAreCompatible(left: string, right: string): boolean {
  return (
    left === right ||
    (left === 'integer' && right === 'float') ||
    (left === 'float' && right === 'integer')
  );
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

function isStrictJunctionTable(
  tableName: string,
  table: SchemaTable,
  tables: Record<string, SchemaTable>
): boolean {
  const columnNames = Object.keys(table.columns ?? {});
  const foreignKeys = Object.entries(table.foreign_keys ?? {});
  if (table.primary_key || columnNames.length !== 2 || foreignKeys.length !== 2) {
    return false;
  }
  if (!columnNames.every((columnName) => table.foreign_keys?.[columnName])) {
    return false;
  }

  const targetTables = foreignKeys.map(([, relation]) => relation.target_table);
  if (targetTables[0] === targetTables[1]) {
    return false;
  }

  return foreignKeys.every(([, relation]) => {
    if (relation.target_table === tableName) {
      return false;
    }
    const targetTable = tables[relation.target_table];
    return Boolean(targetTable?.primary_key) && relation.target_column === targetTable.primary_key;
  });
}

function deriveManyToManyTables(tables: Record<string, SchemaTable>) {
  for (const table of Object.values(tables)) {
    table.many_to_many = {};
  }

  for (const [throughTableName, throughTable] of Object.entries(tables)) {
    if (!isStrictJunctionTable(throughTableName, throughTable, tables)) {
      continue;
    }

    const foreignKeys = Object.entries(throughTable.foreign_keys ?? {}).sort(([left], [right]) =>
      left.localeCompare(right)
    );
    if (foreignKeys.length !== 2) {
      continue;
    }

    const [[leftColumn, leftRelation], [rightColumn, rightRelation]] = foreignKeys;
    addManyToManyRelation(tables, leftRelation.target_table, {
      through_table: throughTableName,
      source_column: leftColumn,
      source_target_column: leftRelation.target_column,
      target_table: rightRelation.target_table,
      target_column: rightRelation.target_column,
      through_target_column: rightColumn
    });
    addManyToManyRelation(tables, rightRelation.target_table, {
      through_table: throughTableName,
      source_column: rightColumn,
      source_target_column: rightRelation.target_column,
      target_table: leftRelation.target_table,
      target_column: leftRelation.target_column,
      through_target_column: leftColumn
    });
  }
}

function addManyToManyRelation(
  tables: Record<string, SchemaTable>,
  sourceTableName: string,
  relation: SchemaManyToManyRelation
) {
  const sourceTable = tables[sourceTableName];
  if (!sourceTable) {
    return;
  }

  const existing = sourceTable.many_to_many ?? {};
  const relationName = existing[relation.target_table]
    ? `${relation.target_table}_via_${relation.through_table}`
    : relation.target_table;
  sourceTable.many_to_many = {
    ...existing,
    [relationName]: relation
  };
}
