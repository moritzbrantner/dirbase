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

import { buildSchemaHandleId } from '../schemaEditor';
import type { OverviewColumn, OverviewEdge, OverviewPageData, ResourceOverview } from '../types';

interface RelationNodeData extends Record<string, unknown> {
  resource: ResourceOverview;
  connectable: boolean;
  selected: boolean;
}

type RelationFlowNode = Node<RelationNodeData, 'schemaTable'>;

const RELATION_NODE_TYPES = {
  schemaTable: RelationNodeCard
};

export function RelationMap({
  overview,
  schemaEdges,
  selectedResourceName,
  onSelectResource,
  onCreateRelationship,
  connectable,
  loading
}: {
  overview: OverviewPageData | null;
  schemaEdges: OverviewEdge[];
  selectedResourceName: string | null;
  onSelectResource: (resourceName: string) => void;
  onCreateRelationship: (connection: Connection) => void;
  connectable: boolean;
  loading: boolean;
}) {
  if (loading) {
    return <div className="skeleton relation-map-skeleton" />;
  }
  if (!overview || overview.resources.length === 0) {
    return <p className="overview-empty">No resources are available yet.</p>;
  }

  const columns = Math.max(1, Math.ceil(Math.sqrt(overview.resources.length)));
  const columnHeights = Array.from({ length: columns }, () => 0);
  const nodes: RelationFlowNode[] = overview.resources.map((resource, index) => {
    const columnIndex = index % columns;
    const nodeHeight = estimateRelationNodeHeight(resource);
    const position = {
      x: columnIndex * 320,
      y: columnHeights[columnIndex]
    };
    columnHeights[columnIndex] += nodeHeight + 36;

    return {
      id: resource.name,
      type: 'schemaTable',
      position,
      draggable: false,
      data: {
        resource,
        connectable,
        selected: resource.name === selectedResourceName
      },
      selectable: false
    };
  });

  const edges: Edge[] = schemaEdges.map((edge) => ({
    id: `${edge.source_table}:${edge.source_column}:${edge.target_table}:${edge.target_column}`,
    source: edge.source_table,
    target: edge.target_table,
    sourceHandle:
      edge.kind === 'many_to_many' ? undefined : buildSchemaHandleId('source', edge.source_column),
    targetHandle:
      edge.kind === 'many_to_many' ? undefined : buildSchemaHandleId('target', edge.target_column),
    label:
      edge.kind === 'many_to_many' && edge.through_table
        ? `via ${edge.through_table}`
        : `${edge.source_column} -> ${edge.target_column}`,
    animated: edge.source_table === selectedResourceName || edge.target_table === selectedResourceName,
    style:
      edge.kind === 'many_to_many'
        ? {
            stroke: edge.source_table === selectedResourceName || edge.target_table === selectedResourceName
              ? '#0b7285'
              : 'rgba(14, 116, 144, 0.55)',
            strokeWidth: edge.source_table === selectedResourceName || edge.target_table === selectedResourceName ? 2.6 : 2,
            strokeDasharray: '7 5'
          }
        : edge.source_table === selectedResourceName || edge.target_table === selectedResourceName
          ? { stroke: '#0f766e', strokeWidth: 2.4 }
          : { stroke: 'rgba(94, 109, 104, 0.42)', strokeWidth: 1.8 },
    labelStyle: { fill: '#39554d', fontWeight: 600 }
  }));

  return (
    <div className="relation-map-shell" data-testid="relation-map">
      <ReactFlow
        fitView
        nodes={nodes}
        edges={edges}
        nodeTypes={RELATION_NODE_TYPES}
        nodesConnectable={connectable}
        elementsSelectable={false}
        onConnect={onCreateRelationship}
        onNodeClick={(_, node) => onSelectResource(node.id)}
      >
        <MiniMap zoomable pannable className="relation-map-minimap" />
        <Controls showInteractive={false} />
        <Background gap={20} size={1} color="rgba(57, 85, 77, 0.12)" />
      </ReactFlow>
    </div>
  );
}

function RelationNodeCard({ data }: NodeProps<RelationFlowNode>) {
  const columns = getRelationNodeColumns(data.resource);

  return (
    <div className={`graph-node-card ${data.selected ? 'is-selected' : ''}`}>
      <div className="graph-node-head">
        <strong>{data.resource.name}</strong>
        <span className="overview-kind-badge">{data.resource.kind}</span>
      </div>
      <p>
        {data.resource.row_count !== null
          ? `${data.resource.row_count} rows`
          : data.resource.key_count !== null
            ? `${data.resource.key_count} keys`
            : 'scalar value'}
      </p>
      {columns.length > 0 ? (
        <div className="graph-node-columns">
          {columns.map((column) => (
            <div
              key={column.name}
              className={`graph-column-row ${column.is_primary_key ? 'is-primary' : ''}`}
            >
              {data.connectable ? (
                <Handle
                  type="target"
                  position={Position.Left}
                  id={buildSchemaHandleId('target', column.name)}
                  className="graph-column-handle is-target"
                />
              ) : null}
              <div className="graph-column-copy">
                <span className="graph-column-name">{column.name}</span>
                <span className="graph-column-meta">
                  {column.column_type}
                  {column.is_primary_key ? ' · pk' : ''}
                  {column.relation ? ' · fk' : ''}
                </span>
              </div>
              {data.connectable ? (
                <Handle
                  type="source"
                  position={Position.Right}
                  id={buildSchemaHandleId('source', column.name)}
                  className="graph-column-handle is-source"
                />
              ) : null}
            </div>
          ))}
        </div>
      ) : (
        <div className="graph-node-empty">No schema columns available for connection editing.</div>
      )}
    </div>
  );
}

function getRelationNodeColumns(resource: ResourceOverview): OverviewColumn[] {
  if (resource.columns.length > 0) {
    return resource.columns;
  }

  return resource.field_names.map((fieldName) => ({
    name: fieldName,
    column_type: 'unknown',
    nullable: true,
    relation: null,
    is_primary_key: resource.primary_key === fieldName
  }));
}

function estimateRelationNodeHeight(resource: ResourceOverview): number {
  const columnCount = getRelationNodeColumns(resource).length;
  return 96 + columnCount * 34;
}
