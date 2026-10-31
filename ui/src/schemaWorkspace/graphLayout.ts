import { deriveSchemaGraphRelations, deriveSchemaGraphTables } from './graph';
import type { ResourceOverview, SchemaTable } from '../types';

const SCHEMA_GRAPH_NODE_WIDTH = 260;
const SCHEMA_GRAPH_NODE_GAP_X = 88;
const SCHEMA_GRAPH_NODE_GAP_Y = 40;
const SCHEMA_GRAPH_MAX_ROWS_PER_LANE = 6;

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
