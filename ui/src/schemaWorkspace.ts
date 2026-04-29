import type { Connection } from '@xyflow/react';

import { isRecord } from './helpers';
import { parseSchemaConnection } from './schemaEditor';
import type {
  DeclaredSchemaResponse,
  DeclaredSchemaTable,
  ResourceOverview,
  SchemaColumn,
  SchemaColumnOverrideInput,
  SchemaEditorPayload,
  SchemaForeignKey,
  SchemaManyToManyRelation,
  SchemaResponse,
  SchemaTable
} from './types';

const EMPTY_DECLARED_SCHEMA: DeclaredSchemaResponse = { tables: {} };
const COLUMN_TYPES = new Set([
  'integer',
  'float',
  'boolean',
  'string',
  'json',
  'date',
  'datetime',
  'uuid',
  'big_integer',
  'decimal'
]);
const TABLE_KINDS = new Set(['object', 'relation', 'unknown']);
const SCHEMA_GRAPH_NODE_WIDTH = 260;
const SCHEMA_GRAPH_NODE_GAP_X = 88;
const SCHEMA_GRAPH_NODE_GAP_Y = 40;
const SCHEMA_GRAPH_MAX_ROWS_PER_LANE = 6;

export interface SchemaGraphColumn {
  name: string;
  column_type: string;
  nullable: boolean;
  relation: 'foreign' | 'one_to_many' | null;
  is_primary_key: boolean;
  canSource: boolean;
  canTarget: boolean;
  visible: boolean;
}

export interface SchemaGraphTable {
  columns: SchemaGraphColumn[];
}

export interface SchemaGraphRelation {
  kind: 'foreign' | 'one_to_many';
  sourceTable: string;
  sourceColumn: string;
  targetTable: string;
  targetColumn: string;
}

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

export function deriveSchemaGraphTables(
  resources: ResourceOverview[],
  effectiveTables: Record<string, SchemaTable>
): Record<string, SchemaGraphTable> {
  const knownTableNames = new Set(resources.map((resource) => resource.name));
  const relations = deriveSchemaGraphRelations(resources, effectiveTables);
  const relationKindsByTable = new Map<string, Map<string, SchemaGraphColumn['relation']>>();
  for (const relation of relations) {
    const columns = relationKindsByTable.get(relation.sourceTable) ?? new Map();
    columns.set(relation.sourceColumn, relation.kind);
    relationKindsByTable.set(relation.sourceTable, columns);
  }
  const baseColumnsByTable = Object.fromEntries(
    resources.map((resource) => {
      const table = normalizeSchemaTable(effectiveTables[resource.name]);
      return [
        resource.name,
        getBaseSchemaGraphColumns(resource, table, relationKindsByTable.get(resource.name))
      ];
    })
  );
  const primaryKeys = resources.flatMap((resource) => {
    const columns = baseColumnsByTable[resource.name] ?? [];
    return columns
      .filter((column) => column.is_primary_key && isKnownComparableColumnType(column.column_type))
      .map((column) => ({
        tableName: resource.name,
        columnName: column.name,
        columnType: column.column_type
      }));
  });
  const targetedColumns = new Map<string, Set<string>>();

  for (const relation of relations) {
    if (!knownTableNames.has(relation.targetTable)) {
      continue;
    }
    const columns = targetedColumns.get(relation.targetTable) ?? new Set<string>();
    columns.add(relation.targetColumn);
    targetedColumns.set(relation.targetTable, columns);
  }

  return Object.fromEntries(
    resources.map((resource) => {
      const columns = (baseColumnsByTable[resource.name] ?? []).map((column) => {
        const isExistingTarget = targetedColumns.get(resource.name)?.has(column.name) ?? false;
        const hasCompatiblePrimaryTarget =
          !column.is_primary_key &&
          isKnownComparableColumnType(column.column_type) &&
          primaryKeys.some(
            (target) =>
              (target.tableName !== resource.name || target.columnName !== column.name) &&
              columnTypesAreCompatible(column.column_type, target.columnType)
          );
        const hasCompatibleSource =
          column.is_primary_key &&
          isKnownComparableColumnType(column.column_type) &&
          resources.some((candidateResource) => {
            const candidateColumns = baseColumnsByTable[candidateResource.name] ?? [];
            return candidateColumns.some(
              (candidateColumn) =>
                !candidateColumn.is_primary_key &&
                isKnownComparableColumnType(candidateColumn.column_type) &&
                columnTypesAreCompatible(candidateColumn.column_type, column.column_type)
            );
          });
        const canSource =
          column.relation === 'foreign'
            ? true
            : column.relation === 'one_to_many'
              ? false
              : hasCompatiblePrimaryTarget;
        const canTarget = column.is_primary_key && (hasCompatibleSource || isExistingTarget);
        return {
          ...column,
          canSource,
          canTarget,
          visible: canSource || canTarget || Boolean(column.relation) || isExistingTarget
        };
      });

      return [
        resource.name,
        {
          columns: columns
            .filter((column) => column.visible)
            .sort((left, right) => {
              if (left.is_primary_key !== right.is_primary_key) {
                return left.is_primary_key ? -1 : 1;
              }
              if (Boolean(left.relation) !== Boolean(right.relation)) {
                return left.relation ? -1 : 1;
              }
              if (left.relation !== right.relation) {
                return left.relation === 'foreign' ? -1 : 1;
              }
              return left.name.localeCompare(right.name);
            })
        }
      ];
    })
  );
}

export function getSchemaGraphAutoLayout(
  resources: ResourceOverview[],
  effectiveTables: Record<string, SchemaTable>,
  options?: { minimizedTables?: Iterable<string> }
): Record<string, { x: number; y: number }> {
  const graphTables = deriveSchemaGraphTables(resources, effectiveTables);
  const minimizedTables = new Set(options?.minimizedTables ?? []);
  const tableNames = resources.map((resource) => resource.name);
  const groupedByRank = rankSchemaGraphTables(resources, effectiveTables);
  const positions: Record<string, { x: number; y: number }> = {};
  let xCursor = 0;

  for (const rankGroup of groupedByRank) {
    const lanes = Math.max(1, Math.ceil(rankGroup.length / SCHEMA_GRAPH_MAX_ROWS_PER_LANE));
    const laneHeights = Array.from({ length: lanes }, () => 0);

    rankGroup.forEach((tableName, index) => {
      const laneIndex = index % lanes;
      const columns = graphTables[tableName]?.columns ?? [];
      positions[tableName] = {
        x: xCursor + laneIndex * (SCHEMA_GRAPH_NODE_WIDTH + SCHEMA_GRAPH_NODE_GAP_X),
        y: laneHeights[laneIndex]
      };
      laneHeights[laneIndex] +=
        estimateSchemaNodeHeight(columns.length, minimizedTables.has(tableName)) +
        SCHEMA_GRAPH_NODE_GAP_Y;
    });

    xCursor += lanes * (SCHEMA_GRAPH_NODE_WIDTH + SCHEMA_GRAPH_NODE_GAP_X) + SCHEMA_GRAPH_NODE_GAP_X;
  }

  for (const tableName of tableNames) {
    if (!positions[tableName]) {
      positions[tableName] = { x: xCursor, y: 0 };
      xCursor += SCHEMA_GRAPH_NODE_WIDTH + SCHEMA_GRAPH_NODE_GAP_X;
    }
  }

  return positions;
}

export function deriveSchemaGraphRelations(
  resources: ResourceOverview[],
  effectiveTables: Record<string, SchemaTable>
): SchemaGraphRelation[] {
  const resourceNames = resources.map((resource) => resource.name);
  const knownTableNames = new Set(resourceNames);
  const aliases = buildSchemaGraphTableAliases(resourceNames);
  const relations = new Map<string, SchemaGraphRelation>();

  for (const sourceTable of resourceNames) {
    const table = normalizeSchemaTable(effectiveTables[sourceTable]);

    for (const [sourceColumn, foreignKey] of Object.entries(table.foreign_keys ?? {})) {
      if (!knownTableNames.has(foreignKey.target_table)) {
        continue;
      }

      relations.set(`${sourceTable}:${sourceColumn}`, {
        kind: 'foreign',
        sourceTable,
        sourceColumn,
        targetTable: foreignKey.target_table,
        targetColumn: foreignKey.target_column
      });
    }

    for (const sourceColumn of Object.keys(table.columns ?? {})) {
      const key = `${sourceTable}:${sourceColumn}`;
      if (relations.has(key)) {
        continue;
      }

      const relation = inferOneToManyGraphRelation(
        sourceTable,
        sourceColumn,
        effectiveTables,
        aliases
      );
      if (relation) {
        relations.set(key, relation);
      }
    }
  }

  return [...relations.values()].sort((left, right) => {
    const leftKey = `${left.sourceTable}:${left.sourceColumn}:${left.targetTable}:${left.targetColumn}`;
    const rightKey = `${right.sourceTable}:${right.sourceColumn}:${right.targetTable}:${right.targetColumn}`;
    return leftKey.localeCompare(rightKey);
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

function isDeclaredTableEmpty(table: DeclaredSchemaTable): boolean {
  return (
    !table.kind &&
    !table.primary_key &&
    !table.columns &&
    !table.foreign_keys &&
    !table.suppressed_foreign_keys &&
    !table.unique
  );
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
    (left === 'float' && right === 'integer') ||
    (isNumericLikeColumnType(left) && isNumericLikeColumnType(right))
  );
}

function isKnownComparableColumnType(columnType: string): boolean {
  return COLUMN_TYPES.has(columnType);
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

function isNumericLikeColumnType(columnType: string): boolean {
  return ['integer', 'float', 'big_integer', 'decimal'].includes(columnType);
}

function isBoundedColumnType(columnType: string): boolean {
  return isNumericLikeColumnType(columnType) || ['date', 'datetime'].includes(columnType);
}

function isStringBackedColumnType(columnType: string): boolean {
  return ['string', 'date', 'datetime', 'uuid', 'big_integer', 'decimal'].includes(columnType);
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

function buildSchemaGraphTableAliases(tableNames: string[]): Map<string, string> {
  const aliases = new Map<string, string>();

  for (const tableName of tableNames) {
    const singular = singularizeSchemaGraphTableName(tableName);
    for (const alias of [
      tableName,
      singular,
      normalizeSchemaGraphAlias(tableName),
      normalizeSchemaGraphAlias(singular)
    ]) {
      if (alias) {
        aliases.set(alias, tableName);
      }
    }
  }

  return aliases;
}

function singularizeSchemaGraphTableName(tableName: string): string {
  if (tableName.endsWith('ies') && tableName.length > 3) {
    return `${tableName.slice(0, -3)}y`;
  }
  if (tableName.endsWith('s') && tableName.length > 1) {
    return tableName.slice(0, -1);
  }
  return tableName;
}

function normalizeSchemaGraphAlias(value: string): string {
  return value.trim().replace(/[-_]/g, '').toLowerCase();
}

function inferOneToManyGraphRelation(
  sourceTable: string,
  sourceColumn: string,
  tables: Record<string, SchemaTable>,
  aliases: Map<string, string>
): SchemaGraphRelation | null {
  const rawTargetAlias = sourceColumn.endsWith('_ids')
    ? sourceColumn.slice(0, -4)
    : sourceColumn.endsWith('Ids') && sourceColumn.length > 3
      ? sourceColumn.slice(0, -3)
      : null;
  if (!rawTargetAlias) {
    return null;
  }

  const targetTableName =
    aliases.get(rawTargetAlias) ?? aliases.get(normalizeSchemaGraphAlias(rawTargetAlias)) ?? null;
  if (!targetTableName || targetTableName === sourceTable) {
    return null;
  }

  const targetTable = normalizeSchemaTable(tables[targetTableName]);
  if (!targetTable.primary_key || !targetTable.columns?.[targetTable.primary_key]) {
    return null;
  }

  return {
    kind: 'one_to_many',
    sourceTable,
    sourceColumn,
    targetTable: targetTableName,
    targetColumn: targetTable.primary_key
  };
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

function getBaseSchemaGraphColumns(
  resource: ResourceOverview,
  table: SchemaTable,
  relationKinds: Map<string, SchemaGraphColumn['relation']> | undefined
): Array<Omit<SchemaGraphColumn, 'canSource' | 'canTarget' | 'visible'>> {
  const resolveRelation = (
    columnName: string,
    fallback: SchemaGraphColumn['relation'] = null
  ): SchemaGraphColumn['relation'] => relationKinds?.get(columnName) ?? fallback;
  const tableColumns = table.columns ?? {};
  const columnNames = Object.keys(tableColumns);
  if (columnNames.length > 0) {
    return columnNames.map((columnName) => ({
      name: columnName,
      column_type: tableColumns[columnName]?.column_type ?? 'unknown',
      nullable: tableColumns[columnName]?.nullable ?? true,
      relation: resolveRelation(columnName),
      is_primary_key: table.primary_key === columnName
    }));
  }

  return resource.columns.length > 0
    ? resource.columns.map((column) => ({
        name: column.name,
        column_type: column.column_type,
        nullable: column.nullable,
        relation: resolveRelation(column.name, column.relation === 'foreign' ? 'foreign' : null),
        is_primary_key: table.primary_key ? table.primary_key === column.name : column.is_primary_key
      }))
    : resource.field_names.map((fieldName) => ({
        name: fieldName,
        column_type: 'unknown',
        nullable: true,
        relation: resolveRelation(fieldName),
        is_primary_key: table.primary_key === fieldName || resource.primary_key === fieldName
      }));
}

function rankSchemaGraphTables(
  resources: ResourceOverview[],
  effectiveTables: Record<string, SchemaTable>
): string[][] {
  const tableNames = resources.map((resource) => resource.name);
  const incomingCounts = new Map<string, number>(tableNames.map((tableName) => [tableName, 0]));
  const outgoing = new Map<string, Set<string>>(tableNames.map((tableName) => [tableName, new Set()]));
  const ranks = new Map<string, number>(tableNames.map((tableName) => [tableName, 0]));
  const relations = deriveSchemaGraphRelations(resources, effectiveTables);

  for (const relation of relations) {
    const neighbors = outgoing.get(relation.sourceTable) ?? new Set<string>();
    if (neighbors.has(relation.targetTable)) {
      continue;
    }
    neighbors.add(relation.targetTable);
    outgoing.set(relation.sourceTable, neighbors);
    incomingCounts.set(relation.targetTable, (incomingCounts.get(relation.targetTable) ?? 0) + 1);
  }

  const queue = tableNames.filter((tableName) => (incomingCounts.get(tableName) ?? 0) === 0).sort();
  const visited = new Set<string>();

  while (queue.length > 0) {
    const tableName = queue.shift();
    if (!tableName || visited.has(tableName)) {
      continue;
    }
    visited.add(tableName);
    const nextRank = (ranks.get(tableName) ?? 0) + 1;

    [...(outgoing.get(tableName) ?? [])]
      .sort()
      .forEach((targetTable) => {
        incomingCounts.set(targetTable, (incomingCounts.get(targetTable) ?? 1) - 1);
        ranks.set(targetTable, Math.max(ranks.get(targetTable) ?? 0, nextRank));
        if ((incomingCounts.get(targetTable) ?? 0) === 0) {
          queue.push(targetTable);
          queue.sort();
        }
      });
  }

  const graphTables = deriveSchemaGraphTables(resources, effectiveTables);
  const relationCounts = relations.reduce<Map<string, number>>((counts, relation) => {
    counts.set(relation.sourceTable, (counts.get(relation.sourceTable) ?? 0) + 1);
    return counts;
  }, new Map());
  const degreeByTable = new Map(
    tableNames.map((tableName) => [
      tableName,
      (relationCounts.get(tableName) ?? 0) + [...(outgoing.get(tableName) ?? [])].length
    ])
  );

  for (const tableName of tableNames) {
    if (!visited.has(tableName)) {
      ranks.set(tableName, 0);
    }
  }

  const grouped = new Map<number, string[]>();
  for (const tableName of tableNames) {
    const rank = ranks.get(tableName) ?? 0;
    const tablesAtRank = grouped.get(rank) ?? [];
    tablesAtRank.push(tableName);
    grouped.set(rank, tablesAtRank);
  }

  return [...grouped.entries()]
    .sort(([left], [right]) => left - right)
    .map(([, group]) =>
      group.sort((left, right) => {
        const leftVisibleColumns = graphTables[left]?.columns.length ?? 0;
        const rightVisibleColumns = graphTables[right]?.columns.length ?? 0;
        const degreeDelta = (degreeByTable.get(right) ?? 0) - (degreeByTable.get(left) ?? 0);
        if (degreeDelta !== 0) {
          return degreeDelta;
        }
        if (rightVisibleColumns !== leftVisibleColumns) {
          return rightVisibleColumns - leftVisibleColumns;
        }
        return left.localeCompare(right);
      })
    );
}

function estimateSchemaNodeHeight(columnCount: number, minimized = false): number {
  if (minimized) {
    return 92;
  }

  return 120 + columnCount * 42;
}
