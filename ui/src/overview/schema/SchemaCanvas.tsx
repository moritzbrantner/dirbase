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
  useUpdateNodeInternals,
  useNodesState
} from '@xyflow/react';

import { buildSchemaHandleId } from '../../schemaEditor';
import {
  columnTypesAreCompatible,
  deriveSchemaGraphRelations,
  deriveSchemaGraphTables,
  getSchemaGraphAutoLayout
} from '../../schemaWorkspace';
import type { ResourceOverview, SchemaTable, SchemaWorkspaceSelection } from '../../types';

interface SchemaNodeData extends Record<string, unknown> {
  resource: ResourceOverview;
  table: SchemaTable;
  columns: ReturnType<typeof deriveSchemaGraphTables>[string]['columns'];
  relationCount: number;
  minimized: boolean;
  selected: boolean;
  selectedColumn: string | null;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
  onToggleMinimized: (tableName: string) => void;
}

type SchemaFlowNode = Node<SchemaNodeData, 'schemaWorkspaceTable'>;
type SchemaFlowEdge = Edge<{ relationKind: 'foreign' | 'one_to_many' }>;

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
  const graphRelations = useMemo(
    () => deriveSchemaGraphRelations(resources, effectiveTables),
    [effectiveTables, resources]
  );
  const relationCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const relation of graphRelations) {
      counts.set(relation.sourceTable, (counts.get(relation.sourceTable) ?? 0) + 1);
    }
    return counts;
  }, [graphRelations]);
  const [minimizedNodeIds, setMinimizedNodeIds] = useState<string[]>([]);
  const minimizedNodeSet = useMemo(() => new Set(minimizedNodeIds), [minimizedNodeIds]);
  const autoLayout = useMemo(
    () =>
      getSchemaGraphAutoLayout(resources, effectiveTables, {
        minimizedTables: minimizedNodeSet
      }),
    [effectiveTables, minimizedNodeSet, resources]
  );
  const [flow, setFlow] = useState<ReactFlowInstance<SchemaFlowNode, SchemaFlowEdge> | null>(null);
  const [nodes, setNodes, onNodesChange] = useNodesState<SchemaFlowNode>([]);

  useEffect(() => {
    setMinimizedNodeIds((current) =>
      current.filter((tableName) => resources.some((resource) => resource.name === tableName))
    );
  }, [resources]);

  useEffect(() => {
    setNodes((currentNodes) => {
      const existingPositions = new Map(currentNodes.map((node) => [node.id, node.position]));
      return resources.map((resource) => buildSchemaNode({
        resource,
        table: effectiveTables[resource.name] ?? {},
        columns: graphTables[resource.name]?.columns ?? [],
        relationCount: relationCounts.get(resource.name) ?? 0,
        minimized: minimizedNodeSet.has(resource.name),
        selection,
        onSelectTable,
        onSelectColumn,
        onToggleMinimized: (tableName) =>
          setMinimizedNodeIds((current) =>
            current.includes(tableName)
              ? current.filter((currentTableName) => currentTableName !== tableName)
              : [...current, tableName]
          ),
        position: existingPositions.get(resource.name) ?? autoLayout[resource.name] ?? { x: 0, y: 0 }
      }));
    });
  }, [autoLayout, effectiveTables, graphTables, minimizedNodeSet, onSelectColumn, onSelectTable, relationCounts, resources, selection, setNodes]);

  useEffect(() => {
    setNodes((currentNodes) =>
      currentNodes.map((node) => ({
        ...node,
        position: autoLayout[node.id] ?? node.position
      }))
    );
    flow?.fitView({ duration: 240, padding: 0.12 });
  }, [autoArrangeNonce, autoLayout, flow, setNodes]);

  const edges: SchemaFlowEdge[] = graphRelations.map((relation) => {
    const selected =
      relation.kind === 'foreign'
        ? selection?.kind === 'relation' &&
          selection.tableName === relation.sourceTable &&
          selection.relationSourceColumn === relation.sourceColumn
        : selection?.kind === 'column' &&
          selection.tableName === relation.sourceTable &&
          selection.columnName === relation.sourceColumn;

    return {
      id: `${relation.sourceTable}:${relation.sourceColumn}:${relation.targetTable}:${relation.targetColumn}`,
      source: relation.sourceTable,
      target: relation.targetTable,
      sourceHandle: buildSchemaHandleId('source', relation.sourceColumn),
      targetHandle: buildSchemaHandleId('target', relation.targetColumn),
      label:
        relation.kind === 'one_to_many'
          ? `${relation.sourceColumn} -> ${relation.targetColumn}[]`
          : `${relation.sourceColumn} -> ${relation.targetColumn}`,
      animated: selected,
      style:
        relation.kind === 'one_to_many'
          ? selected
            ? { stroke: '#0f766e', strokeWidth: 2.6, strokeDasharray: '8 5' }
            : { stroke: 'rgba(15, 118, 110, 0.55)', strokeWidth: 1.9, strokeDasharray: '8 5' }
          : selected
            ? { stroke: '#0891b2', strokeWidth: 2.6 }
            : { stroke: 'rgba(46, 84, 104, 0.52)', strokeWidth: 1.8 },
      labelStyle: { fill: '#214154', fontWeight: 600 },
      data: { relationKind: relation.kind }
    };
  });
  const connectable = !readonly && !structuredEditingDisabled;

  return (
    <div className="schema-canvas-shell" data-testid="schema-canvas">
      <ReactFlow<SchemaFlowNode, SchemaFlowEdge>
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
          if (edge.data?.relationKind === 'foreign') {
            onSelectRelation(tableName, sourceColumn);
            return;
          }
          onSelectColumn(tableName, sourceColumn);
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
  const updateNodeInternals = useUpdateNodeInternals();

  useEffect(() => {
    updateNodeInternals(data.resource.name);
  }, [data.columns, data.minimized, data.resource.name, updateNodeInternals]);

  return (
    <div className={`schema-node-card ${data.selected ? 'is-selected' : ''} ${data.minimized ? 'is-minimized' : ''}`}>
      <div className="schema-node-head">
        <button type="button" className="schema-node-title" onClick={() => data.onSelectTable(data.resource.name)}>
          {data.resource.name}
        </button>
        <div className="schema-node-head-actions">
          <span className="overview-kind-badge">{data.table.kind ?? data.resource.kind}</span>
          <button
            type="button"
            className="schema-node-toggle"
            aria-label={data.minimized ? `Expand ${data.resource.name}` : `Minify ${data.resource.name}`}
            onClick={(event) => {
              event.stopPropagation();
              data.onToggleMinimized(data.resource.name);
            }}
          >
            {data.minimized ? 'Expand' : 'Minify'}
          </button>
        </div>
      </div>
      <div className="schema-node-summary">
        <span>{data.resource.row_count !== null ? `${data.resource.row_count} rows` : 'resource'}</span>
        <span>{data.relationCount} relations</span>
      </div>
      {data.minimized ? (
        <div className="schema-node-minified-copy">
          {columns.length === 1 ? '1 compatible column hidden' : `${columns.length} compatible columns hidden`}
        </div>
      ) : (
        <div className="schema-node-columns">
          {columns.length > 0 ? (
            columns.map((column) => {
              const selected = data.selectedColumn === column.name;
              return (
                <div key={column.name} className={`schema-node-column ${selected ? 'is-selected' : ''}`}>
                  {column.canTarget ? (
                    <Handle
                      type="target"
                      position={Position.Left}
                      id={buildSchemaHandleId('target', column.name)}
                      className="graph-column-handle is-target"
                      isConnectable
                      data-testid={`${data.resource.name}:${column.name}:target-handle`}
                    />
                  ) : null}
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
                      {column.relation === 'foreign'
                        ? ' · fk'
                        : column.relation === 'one_to_many'
                          ? ' · 1:n'
                          : ''}
                    </span>
                  </button>
                  {column.canSource ? (
                    <Handle
                      type="source"
                      position={Position.Right}
                      id={buildSchemaHandleId('source', column.name)}
                      className="graph-column-handle is-source"
                      isConnectable
                      data-testid={`${data.resource.name}:${column.name}:source-handle`}
                    />
                  ) : null}
                </div>
              );
            })
          ) : (
            <div className="graph-node-empty">No compatible key columns are available in this table.</div>
          )}
        </div>
      )}
    </div>
  );
}

function buildSchemaNode({
  resource,
  table,
  columns,
  relationCount,
  minimized,
  selection,
  onSelectTable,
  onSelectColumn,
  onToggleMinimized,
  position
}: {
  resource: ResourceOverview;
  table: SchemaTable;
  columns: ReturnType<typeof deriveSchemaGraphTables>[string]['columns'];
  relationCount: number;
  minimized: boolean;
  selection: SchemaWorkspaceSelection | null;
  onSelectTable: (tableName: string) => void;
  onSelectColumn: (tableName: string, columnName: string) => void;
  onToggleMinimized: (tableName: string) => void;
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
      relationCount,
      minimized,
      selected: selection?.tableName === resource.name,
      selectedColumn:
        selection?.kind === 'column' && selection.tableName === resource.name
          ? selection.columnName
          : selection?.kind === 'relation' && selection.tableName === resource.name
            ? selection.relationSourceColumn
            : null,
      onSelectTable,
      onSelectColumn,
      onToggleMinimized
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
