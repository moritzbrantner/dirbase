import { columnTypesAreCompatible, isKnownComparableColumnType } from './columnTypes';
import { normalizeSchemaTable } from './normalize';
import type { ResourceOverview, SchemaTable } from '../types';

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
