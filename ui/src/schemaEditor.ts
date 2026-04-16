import type { Connection } from '@xyflow/react';

import { isRecord } from './helpers';
import type { OverviewEdge, ResourceOverview, SchemaForeignKey, SchemaResponse, SchemaTable } from './types';

const SOURCE_HANDLE_PREFIX = 'source:';
const TARGET_HANDLE_PREFIX = 'target:';

export interface SchemaConnection {
  sourceTable: string;
  sourceColumn: string;
  targetTable: string;
  targetColumn: string;
}

export function buildSchemaHandleId(kind: 'source' | 'target', columnName: string): string {
  return `${kind === 'source' ? SOURCE_HANDLE_PREFIX : TARGET_HANDLE_PREFIX}${columnName}`;
}

export function parseSchemaConnection(connection: Connection): SchemaConnection | null {
  const sourceColumn = parseSchemaHandle(connection.sourceHandle, SOURCE_HANDLE_PREFIX);
  const targetColumn = parseSchemaHandle(connection.targetHandle, TARGET_HANDLE_PREFIX);

  if (!connection.source || !connection.target || !sourceColumn || !targetColumn) {
    return null;
  }

  return {
    sourceTable: connection.source,
    sourceColumn,
    targetTable: connection.target,
    targetColumn
  };
}

export function parseSchemaDocument(input: string): { document: SchemaResponse | null; error: string | null } {
  if (!input.trim()) {
    return {
      document: { tables: {} },
      error: null
    };
  }

  try {
    const parsed = JSON.parse(input) as unknown;
    if (!isRecord(parsed)) {
      return {
        document: null,
        error: 'Schema draft must be a JSON object before relationships can be edited in the graph.'
      };
    }
    return {
      document: parsed as SchemaResponse,
      error: null
    };
  } catch (error) {
    return {
      document: null,
      error: error instanceof Error ? error.message : 'Schema draft is not valid JSON.'
    };
  }
}

export function upsertSchemaRelationship(schema: SchemaResponse, connection: SchemaConnection): SchemaResponse {
  const tables = cloneTables(schema.tables);
  const sourceTable = cloneTable(tables[connection.sourceTable]);
  const foreignKeys = cloneForeignKeys(sourceTable.foreign_keys);

  foreignKeys[connection.sourceColumn] = {
    target_table: connection.targetTable,
    target_column: connection.targetColumn
  };

  tables[connection.sourceTable] = {
    ...sourceTable,
    foreign_keys: foreignKeys
  };

  return {
    ...schema,
    tables
  };
}

export function deriveSchemaEdges(schema: SchemaResponse, resources: ResourceOverview[]): OverviewEdge[] {
  const resourceNames = new Set(resources.map((resource) => resource.name));
  const edges = new Map<string, OverviewEdge>();

  for (const [sourceTable, tableValue] of Object.entries(schema.tables ?? {})) {
    if (!resourceNames.has(sourceTable)) {
      continue;
    }

    const table = cloneTable(tableValue);
    for (const [sourceColumn, foreignKeyValue] of Object.entries(table.foreign_keys ?? {})) {
      if (!isSchemaForeignKey(foreignKeyValue) || !resourceNames.has(foreignKeyValue.target_table)) {
        continue;
      }

      const key = `${sourceTable}:${sourceColumn}`;
      edges.set(key, {
        source_table: sourceTable,
        source_column: sourceColumn,
        target_table: foreignKeyValue.target_table,
        target_column: foreignKeyValue.target_column
      });
    }
  }

  return [...edges.values()].sort((left, right) => {
    const leftKey = `${left.source_table}:${left.source_column}:${left.target_table}:${left.target_column}`;
    const rightKey = `${right.source_table}:${right.source_column}:${right.target_table}:${right.target_column}`;
    return leftKey.localeCompare(rightKey);
  });
}

function parseSchemaHandle(handleId: string | null, prefix: string): string | null {
  if (!handleId?.startsWith(prefix)) {
    return null;
  }

  const value = handleId.slice(prefix.length).trim();
  return value ? value : null;
}

function cloneTables(value: SchemaResponse['tables']): Record<string, SchemaTable> {
  if (!isRecord(value)) {
    return {};
  }

  return Object.fromEntries(
    Object.entries(value).map(([name, table]) => [name, cloneTable(table)])
  );
}

function cloneTable(value: unknown): SchemaTable {
  if (!isRecord(value)) {
    return {};
  }

  const table = value as SchemaTable;
  return {
    ...table,
    columns: isRecord(table.columns) ? { ...table.columns } : undefined,
    foreign_keys: cloneForeignKeys(table.foreign_keys)
  };
}

function cloneForeignKeys(value: SchemaTable['foreign_keys']): Record<string, SchemaForeignKey> {
  if (!isRecord(value)) {
    return {};
  }

  return Object.fromEntries(
    Object.entries(value).flatMap(([column, foreignKey]) =>
      isSchemaForeignKey(foreignKey) ? [[column, { ...foreignKey }]] : []
    )
  );
}

function isSchemaForeignKey(value: unknown): value is SchemaForeignKey {
  return (
    isRecord(value) &&
    typeof value.target_table === 'string' &&
    typeof value.target_column === 'string'
  );
}
