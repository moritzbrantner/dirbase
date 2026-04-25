import {
  Background,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  type Connection,
  type Edge,
  type Node,
  type NodeProps
} from '@xyflow/react';

import { buildSchemaHandleId } from '../../schemaEditor';
import type { ResourceOverview, SchemaTable, SchemaWorkspaceSelection } from '../../types';

interface SchemaNodeData extends Record<string, unknown> {
  resource: ResourceOverview;
  table: SchemaTable;
  selected: boolean;
  selectedColumn: string | null;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
}

type SchemaFlowNode = Node<SchemaNodeData, 'schemaWorkspaceTable'>;

const NODE_TYPES = {
  schemaWorkspaceTable: SchemaWorkspaceNode
};

export function SchemaCanvas({
  resources,
  effectiveTables,
  selection,
  readonly,
  structuredEditingDisabled,
  onSelectTable,
  onSelectColumn,
  onSelectRelation,
  onCreateRelationship
}: {
  resources: ResourceOverview[];
  effectiveTables: Record<string, SchemaTable>;
  selection: SchemaWorkspaceSelection | null;
  readonly: boolean;
  structuredEditingDisabled: boolean;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
  onSelectRelation: (tableName: string, sourceColumn: string) => void;
  onCreateRelationship: (connection: Connection) => void;
}) {
  const columns = Math.max(1, Math.ceil(Math.sqrt(Math.max(resources.length, 1))));
  const columnHeights = Array.from({ length: columns }, () => 0);

  const nodes: SchemaFlowNode[] = resources.map((resource, index) => {
    const table = effectiveTables[resource.name] ?? {};
    const columnIndex = index % columns;
    const nodeHeight = estimateNodeHeight(resource, table);
    const position = {
      x: columnIndex * 320,
      y: columnHeights[columnIndex]
    };
    columnHeights[columnIndex] += nodeHeight + 28;

    return {
      id: resource.name,
      type: 'schemaWorkspaceTable',
      position,
      draggable: false,
      selectable: false,
      data: {
        resource,
        table,
        selected: selection?.tableName === resource.name,
        selectedColumn:
          selection?.kind === 'column' && selection.tableName === resource.name
            ? selection.columnName
            : selection?.kind === 'relation' && selection.tableName === resource.name
              ? selection.relationSourceColumn
              : null,
        onSelectTable,
        onSelectColumn
      }
    };
  });

  const edges: Edge[] = resources.flatMap((resource) => {
    const table = effectiveTables[resource.name] ?? {};
    return Object.entries(table.foreign_keys ?? {}).map(([sourceColumn, relation]) => {
      const selected =
        selection?.kind === 'relation' &&
        selection.tableName === resource.name &&
        selection.relationSourceColumn === sourceColumn;
      return {
        id: `${resource.name}:${sourceColumn}:${relation.target_table}:${relation.target_column}`,
        source: resource.name,
        target: relation.target_table,
        sourceHandle: buildSchemaHandleId('source', sourceColumn),
        targetHandle: buildSchemaHandleId('target', relation.target_column),
        label: `${sourceColumn} -> ${relation.target_column}`,
        animated: selected,
        style: selected
          ? { stroke: '#0891b2', strokeWidth: 2.6 }
          : { stroke: 'rgba(46, 84, 104, 0.52)', strokeWidth: 1.8 },
        labelStyle: { fill: '#214154', fontWeight: 600 }
      } satisfies Edge;
    });
  });

  return (
    <div className="schema-canvas-shell" data-testid="schema-canvas">
      <ReactFlow
        fitView
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        nodesConnectable={!readonly && !structuredEditingDisabled}
        elementsSelectable={false}
        onConnect={onCreateRelationship}
        onNodeClick={(_, node) => onSelectTable(node.id)}
        onEdgeClick={(_, edge) => {
          const [tableName, sourceColumn] = edge.id.split(':', 2);
          onSelectRelation(tableName, sourceColumn);
        }}
      >
        <MiniMap zoomable pannable className="relation-map-minimap" />
        <Controls showInteractive={false} />
        <Background gap={18} size={1} color="rgba(28, 77, 94, 0.11)" />
      </ReactFlow>
    </div>
  );
}

function SchemaWorkspaceNode({ data }: NodeProps<SchemaFlowNode>) {
  const columns = getNodeColumns(data.resource, data.table);

  return (
    <div className={`schema-node-card ${data.selected ? 'is-selected' : ''}`}>
      <div className="schema-node-head">
        <button type="button" className="schema-node-title" onClick={() => data.onSelectTable(data.resource.name)}>
          {data.resource.name}
        </button>
        <span className="overview-kind-badge">{data.table.kind ?? data.resource.kind}</span>
      </div>
      <div className="schema-node-summary">
        <span>{data.resource.row_count !== null ? `${data.resource.row_count} rows` : 'resource'}</span>
        <span>{Object.keys(data.table.foreign_keys ?? {}).length} relations</span>
      </div>
      <div className="schema-node-columns">
        {columns.map((column) => {
          const selected = data.selectedColumn === column.name;
          return (
            <div key={column.name} className={`schema-node-column ${selected ? 'is-selected' : ''}`}>
              <Handle
                type="target"
                position={Position.Left}
                id={buildSchemaHandleId('target', column.name)}
                className="graph-column-handle is-target"
                data-testid={`${data.resource.name}:${column.name}:target-handle`}
              />
              <button
                type="button"
                className="schema-node-column-button"
                onClick={(event) => {
                  event.stopPropagation();
                  data.onSelectColumn(data.resource.name, column.name);
                }}
              >
                <span className="schema-node-column-name">{column.name}</span>
                <span className="schema-node-column-meta">
                  {column.column_type}
                  {column.is_primary_key ? ' · pk' : ''}
                  {column.relation ? ' · fk' : ''}
                </span>
              </button>
              <Handle
                type="source"
                position={Position.Right}
                id={buildSchemaHandleId('source', column.name)}
                className="graph-column-handle is-source"
                data-testid={`${data.resource.name}:${column.name}:source-handle`}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function getNodeColumns(resource: ResourceOverview, table: SchemaTable) {
  const tableColumns = table.columns ?? {};
  const columnNames = Object.keys(tableColumns);
  if (columnNames.length > 0) {
    return columnNames
      .map((columnName) => ({
        name: columnName,
        column_type: tableColumns[columnName]?.column_type ?? 'string',
        nullable: tableColumns[columnName]?.nullable ?? true,
        relation: table.foreign_keys?.[columnName] ? 'foreign' : null,
        is_primary_key: table.primary_key === columnName
      }))
      .sort((left, right) => {
        if (left.is_primary_key !== right.is_primary_key) {
          return left.is_primary_key ? -1 : 1;
        }
        return left.name.localeCompare(right.name);
      });
  }

  return resource.columns.length > 0
    ? resource.columns
    : resource.field_names.map((fieldName) => ({
        name: fieldName,
        column_type: 'unknown',
        nullable: true,
        relation: null,
        is_primary_key: resource.primary_key === fieldName
      }));
}

function estimateNodeHeight(resource: ResourceOverview, table: SchemaTable): number {
  return 104 + getNodeColumns(resource, table).length * 38;
}
