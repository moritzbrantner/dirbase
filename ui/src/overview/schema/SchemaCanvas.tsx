import { useEffect, useMemo, useState } from 'react';
import {
  Background,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  type ReactFlowInstance,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  useNodesState
} from '@xyflow/react';

import { buildSchemaHandleId } from '../../schemaEditor';
import {
  deriveSchemaGraphTables,
  getSchemaGraphAutoLayout
} from '../../schemaWorkspace';
import type { ResourceOverview, SchemaTable, SchemaWorkspaceSelection } from '../../types';

interface SchemaNodeData extends Record<string, unknown> {
  resource: ResourceOverview;
  table: SchemaTable;
  columns: ReturnType<typeof deriveSchemaGraphTables>[string]['columns'];
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
  autoArrangeNonce,
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
  autoArrangeNonce: number;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
  onSelectRelation: (tableName: string, sourceColumn: string) => void;
  onCreateRelationship: (connection: Connection) => void;
}) {
  const graphTables = useMemo(
    () => deriveSchemaGraphTables(resources, effectiveTables),
    [effectiveTables, resources]
  );
  const autoLayout = useMemo(
    () => getSchemaGraphAutoLayout(resources, effectiveTables),
    [effectiveTables, resources]
  );
  const [flow, setFlow] = useState<ReactFlowInstance<SchemaFlowNode, Edge> | null>(null);
  const [nodes, setNodes, onNodesChange] = useNodesState<SchemaFlowNode>([]);

  useEffect(() => {
    setNodes((currentNodes) => {
      const existingPositions = new Map(currentNodes.map((node) => [node.id, node.position]));
      return resources.map((resource) => buildSchemaNode({
        resource,
        table: effectiveTables[resource.name] ?? {},
        columns: graphTables[resource.name]?.columns ?? [],
        selection,
        onSelectTable,
        onSelectColumn,
        position: existingPositions.get(resource.name) ?? autoLayout[resource.name] ?? { x: 0, y: 0 }
      }));
    });
  }, [autoLayout, effectiveTables, graphTables, onSelectColumn, onSelectTable, resources, selection, setNodes]);

  useEffect(() => {
    setNodes((currentNodes) =>
      currentNodes.map((node) => ({
        ...node,
        position: autoLayout[node.id] ?? node.position
      }))
    );
    flow?.fitView({ duration: 240, padding: 0.12 });
  }, [autoArrangeNonce, autoLayout, flow, setNodes]);

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
  const connectable = !readonly && !structuredEditingDisabled;

  return (
    <div className="schema-canvas-shell" data-testid="schema-canvas">
      <ReactFlow<SchemaFlowNode, Edge>
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        nodesConnectable={connectable}
        nodesDraggable
        elementsSelectable={false}
        onInit={(instance) => setFlow(instance)}
        onNodesChange={onNodesChange}
        onConnect={(connection) => {
          if (isValidSchemaConnection(connection, graphTables)) {
            onCreateRelationship(connection);
          }
        }}
        isValidConnection={(connection) => isValidSchemaConnection(connection, graphTables)}
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
  const columns = data.columns;

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
        {columns.length > 0 ? (
          columns.map((column) => {
            const selected = data.selectedColumn === column.name;
            return (
              <div key={column.name} className={`schema-node-column ${selected ? 'is-selected' : ''}`}>
                <Handle
                  type="target"
                  position={Position.Left}
                  id={buildSchemaHandleId('target', column.name)}
                  className="graph-column-handle is-target"
                  isConnectable={column.canTarget}
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
                  isConnectable={column.canSource}
                  data-testid={`${data.resource.name}:${column.name}:source-handle`}
                />
              </div>
            );
          })
        ) : (
          <div className="graph-node-empty">No compatible key columns are available in this table.</div>
        )}
      </div>
    </div>
  );
}

function buildSchemaNode({
  resource,
  table,
  columns,
  selection,
  onSelectTable,
  onSelectColumn,
  position
}: {
  resource: ResourceOverview;
  table: SchemaTable;
  columns: ReturnType<typeof deriveSchemaGraphTables>[string]['columns'];
  selection: SchemaWorkspaceSelection | null;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
  position: { x: number; y: number };
}): SchemaFlowNode {
  return {
    id: resource.name,
    type: 'schemaWorkspaceTable',
    position,
    draggable: true,
    selectable: false,
    data: {
      resource,
      table,
      columns,
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
}

function isValidSchemaConnection(
  connection: Connection | Edge,
  graphTables: ReturnType<typeof deriveSchemaGraphTables>
): boolean {
  if (!connection.source || !connection.target || !connection.sourceHandle || !connection.targetHandle) {
    return false;
  }

  const sourceColumnName = connection.sourceHandle.replace(/^source:/, '');
  const targetColumnName = connection.targetHandle.replace(/^target:/, '');
  const sourceColumn = graphTables[connection.source]?.columns.find((column) => column.name === sourceColumnName);
  const targetColumn = graphTables[connection.target]?.columns.find((column) => column.name === targetColumnName);

  return Boolean(
    sourceColumn?.canSource &&
      targetColumn?.canTarget &&
      targetColumn.is_primary_key &&
      columnTypesAreCompatible(sourceColumn.column_type, targetColumn.column_type)
  );
}

function columnTypesAreCompatible(left: string, right: string): boolean {
  return left === right || (left === 'integer' && right === 'float') || (left === 'float' && right === 'integer');
}
