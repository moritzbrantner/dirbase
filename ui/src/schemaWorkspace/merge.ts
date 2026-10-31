import {
  cleanSchemaTable,
  normalizeDeclaredSchema,
  normalizeDeclaredTable,
  normalizeSchema,
  normalizeSchemaTable
} from './normalize';
import type {
  DeclaredSchemaResponse,
  SchemaEditorPayload,
  SchemaManyToManyRelation,
  SchemaResponse,
  SchemaTable
} from '../types';

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
