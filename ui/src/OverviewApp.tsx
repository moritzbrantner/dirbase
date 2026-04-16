import { QueryClient, QueryClientProvider, useMutation, useQuery } from '@tanstack/react-query';
import { flexRender, getCoreRowModel, useReactTable, type ColumnDef, type VisibilityState } from '@tanstack/react-table';
import {
  Background,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  type Connection,
  type Edge,
  type Node,
  type NodeProps
} from '@xyflow/react';
import {
  startTransition,
  useDeferredValue,
  useEffect,
  useState,
  type MouseEvent
} from 'react';

import { fetchOverview, fetchResource, fetchSchema, saveSchemaDocument } from './api';
import {
  coerceRelationValue,
  formatJson,
  getColumnNames,
  getTableRows,
  isPaginatedResponse,
  isRecord,
  summarizeValue,
  truncate
} from './helpers';
import {
  buildSchemaHandleId,
  deriveSchemaEdges,
  parseSchemaConnection,
  parseSchemaDocument,
  upsertSchemaRelationship
} from './schemaEditor';
import type {
  FilterDescriptor,
  FilterOperator,
  OverviewColumn,
  OverviewEdge,
  OverviewPageData,
  OverviewRelation,
  OverviewUrlState,
  ResourceOverview,
  ResourceResponse
} from './types';
import {
  FILTER_OPERATORS,
  PAGE_SIZE_OPTIONS,
  buildBrowserQueryString,
  buildResourceRequestPath,
  createFilter,
  nextSorting,
  parseOverviewState,
  resetTableState
} from './urlState';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: false
    }
  }
});

export function OverviewAppRoot({ overviewEndpoint }: { overviewEndpoint: string }) {
  return (
    <QueryClientProvider client={queryClient}>
      <ReactFlowProvider>
        <OverviewApp overviewEndpoint={overviewEndpoint} />
      </ReactFlowProvider>
    </QueryClientProvider>
  );
}

export function OverviewApp({ overviewEndpoint }: { overviewEndpoint: string }) {
  const [urlState, setUrlState] = useState<OverviewUrlState>(() => parseOverviewState(window.location.search));
  const [sidebarSearch, setSidebarSearch] = useState('');
  const deferredSidebarSearch = useDeferredValue(sidebarSearch);
  const [selectedRow, setSelectedRow] = useState<Record<string, unknown> | null>(null);
  const [mobilePanel, setMobilePanel] = useState<'map' | 'data' | 'details'>('data');
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [schemaDraft, setSchemaDraft] = useState('{}');
  const [schemaDraftDirty, setSchemaDraftDirty] = useState(false);
  const [schemaStatus, setSchemaStatus] = useState<string | null>(null);

  const overviewQuery = useQuery({
    queryKey: ['overview', overviewEndpoint],
    queryFn: () => fetchOverview(overviewEndpoint)
  });
  const schemaQuery = useQuery({
    queryKey: ['schema'],
    queryFn: () => fetchSchema()
  });
  const saveSchemaMutation = useMutation({
    mutationFn: saveSchemaDocument,
    onSuccess: async () => {
      setSchemaDraftDirty(false);
      setSchemaStatus('Schema saved.');
      await queryClient.invalidateQueries({ queryKey: ['schema'] });
      await queryClient.invalidateQueries({ queryKey: ['overview'] });
      await queryClient.invalidateQueries({ queryKey: ['resource'] });
    },
    onError: (error) => {
      setSchemaStatus(error instanceof Error ? error.message : 'Schema save failed.');
    }
  });

  useEffect(() => {
    const handlePopState = () => {
      startTransition(() => {
        setUrlState(parseOverviewState(window.location.search));
      });
    };

    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, []);

  useEffect(() => {
    const source = new EventSource('/events');
    const invalidate = () => {
      void queryClient.invalidateQueries({ queryKey: ['overview'] });
      void queryClient.invalidateQueries({ queryKey: ['resource'] });
      void queryClient.invalidateQueries({ queryKey: ['schema'] });
    };
    source.addEventListener('overview_changed', invalidate);
    source.addEventListener('resource_changed', invalidate);
    source.addEventListener('schema_changed', invalidate);
    source.onerror = () => {
      source.close();
    };
    return () => source.close();
  }, []);

  useEffect(() => {
    if (!schemaQuery.data || schemaDraftDirty) {
      return;
    }
    setSchemaDraft(formatJson(schemaQuery.data));
  }, [schemaDraftDirty, schemaQuery.data]);

  const resources = overviewQuery.data?.resources ?? [];
  const parsedSchemaDraft = parseSchemaDocument(schemaDraft);
  const hasUsableSchemaDraft = schemaDraftDirty || Boolean(schemaQuery.data);
  const schemaEdges = hasUsableSchemaDraft && parsedSchemaDraft.document
    ? deriveSchemaEdges(parsedSchemaDraft.document, resources)
    : overviewQuery.data?.edges ?? [];
  const selectedResource =
    resources.find((resource) => resource.name === urlState.resource) ?? resources[0] ?? null;

  useEffect(() => {
    if (!selectedResource) {
      return;
    }
    if (urlState.resource === selectedResource.name) {
      return;
    }
    commitUrlState({ ...resetTableState(selectedResource.name, urlState.view) });
  }, [selectedResource, urlState.resource, urlState.view]);

  useEffect(() => {
    if (!selectedResource || selectedResource.kind === 'table') {
      return;
    }
    if (
      urlState.page !== 1 ||
      urlState.perPage !== 25 ||
      urlState.filters.length > 0 ||
      urlState.sorting.length > 0 ||
      urlState.embeds.length > 0
    ) {
      commitUrlState({
        ...resetTableState(selectedResource.name, urlState.view)
      });
    }
  }, [
    selectedResource,
    urlState.embeds.length,
    urlState.filters.length,
    urlState.page,
    urlState.perPage,
    urlState.sorting.length,
    urlState.view
  ]);

  const requestPath = buildResourceRequestPath(selectedResource, urlState);
  const resourceQuery = useQuery({
    queryKey: ['resource', requestPath],
    queryFn: () => fetchResource(requestPath),
    enabled: Boolean(selectedResource)
  });

  const tableRows = selectedResource?.kind === 'table' ? getTableRows(resourceQuery.data?.parsed) : [];

  useEffect(() => {
    setSelectedRow(null);
  }, [selectedResource?.name, resourceQuery.data?.url]);

  useEffect(() => {
    if (selectedResource?.kind !== 'table' || tableRows.length === 0) {
      return;
    }
    if (!selectedRow) {
      setSelectedRow(tableRows[0] ?? null);
      return;
    }
    const primaryKey = selectedResource.primary_key;
    if (!primaryKey) {
      return;
    }
    const selectedValue = selectedRow[primaryKey];
    const stillExists = tableRows.some((row) => row[primaryKey] === selectedValue);
    if (!stillExists) {
      setSelectedRow(tableRows[0] ?? null);
    }
  }, [selectedResource, selectedRow, tableRows]);

  const filteredResources = resources.filter((resource) => {
    if (!deferredSidebarSearch.trim()) {
      return true;
    }
    const needle = deferredSidebarSearch.trim().toLowerCase();
    return (
      resource.name.toLowerCase().includes(needle) ||
      resource.field_names.some((field) => field.toLowerCase().includes(needle))
    );
  });

  function commitUrlState(nextState: OverviewUrlState) {
    const queryString = buildBrowserQueryString(nextState);
    const nextUrl = `${window.location.pathname}${queryString}`;
    if (nextUrl !== `${window.location.pathname}${window.location.search}`) {
      window.history.replaceState(null, '', nextUrl);
    }
    startTransition(() => {
      setUrlState(nextState);
    });
  }

  function selectResource(resourceName: string, filters: FilterDescriptor[] = []) {
    commitUrlState({
      ...resetTableState(resourceName, urlState.view),
      filters
    });
    setSelectedRow(null);
    setMobilePanel('data');
  }

  function updateTableState(updater: (state: OverviewUrlState) => OverviewUrlState) {
    if (!selectedResource) {
      return;
    }
    commitUrlState(updater({ ...urlState, resource: selectedResource.name }));
  }

  async function copyText(value: string) {
    if (!navigator.clipboard) {
      setCopyStatus('Clipboard is unavailable in this browser.');
      return;
    }
    await navigator.clipboard.writeText(value);
    setCopyStatus('Copied to clipboard.');
    window.setTimeout(() => setCopyStatus(null), 1_500);
  }

  function updateSchemaDraft(nextDraft: string) {
    setSchemaDraft(nextDraft);
    setSchemaDraftDirty(true);
    setSchemaStatus(null);
  }

  function reloadSchemaDraft() {
    if (schemaQuery.data) {
      setSchemaDraft(formatJson(schemaQuery.data));
    }
    setSchemaDraftDirty(false);
    setSchemaStatus('Schema reloaded from the server.');
    void queryClient.invalidateQueries({ queryKey: ['schema'] });
  }

  function handleRelationshipCreate(connection: Connection) {
    const parsedConnection = parseSchemaConnection(connection);
    if (!parsedConnection) {
      setSchemaStatus('Drag from a source column on one table to a target column on another table.');
      return;
    }

    if (!parsedSchemaDraft.document) {
      setSchemaStatus(parsedSchemaDraft.error ?? 'Fix the schema draft before creating relationships from the map.');
      return;
    }

    const nextSchema = upsertSchemaRelationship(parsedSchemaDraft.document, parsedConnection);
    setSchemaDraft(formatJson(nextSchema));
    setSchemaDraftDirty(true);
    setSchemaStatus(
      `Staged ${parsedConnection.sourceTable}.${parsedConnection.sourceColumn} -> ${parsedConnection.targetTable}.${parsedConnection.targetColumn}. Save schema to persist it.`
    );
    setMobilePanel('details');
  }

  const selectedOutgoingLinks = selectedResource && selectedRow
    ? selectedResource.outgoing_relations
        .map((relation) => ({
          relation,
          value: coerceRelationValue(selectedRow, relation)
        }))
        .filter((entry) => entry.value !== null)
    : [];

  return (
    <div className="overview-app-shell">
      <section className="overview-summary-grid">
        <SummaryCard
          label="Resources"
          value={String(overviewQuery.data?.stats.resource_count ?? 0)}
          copy="Tables, objects, and scalar resources currently exposed by the server."
        />
        <SummaryCard
          label="Table links"
          value={String(overviewQuery.data?.stats.relation_count ?? 0)}
          copy="Foreign-key relationships discovered from declared or inferred schema metadata."
        />
        <SummaryCard
          label="Rows"
          value={String(overviewQuery.data?.stats.total_rows ?? 0)}
          copy="Approximate total array rows across table-shaped resources."
        />
        <SummaryCard
          label="Source"
          value={overviewQuery.data?.data_source_kind ?? 'loading'}
          copy={overviewQuery.data?.source_rule ?? 'Loading data source metadata...'}
        />
      </section>

      <div className="mobile-panel-switcher" role="tablist" aria-label="Overview sections">
        <button
          type="button"
          className={mobilePanel === 'map' ? 'is-active' : ''}
          onClick={() => setMobilePanel('map')}
        >
          Map
        </button>
        <button
          type="button"
          className={mobilePanel === 'data' ? 'is-active' : ''}
          onClick={() => setMobilePanel('data')}
        >
          Data
        </button>
        <button
          type="button"
          className={mobilePanel === 'details' ? 'is-active' : ''}
          onClick={() => setMobilePanel('details')}
        >
          Details
        </button>
      </div>

      <section className={`overview-panel shell-card ${mobilePanel === 'map' ? 'mobile-active' : ''}`}>
        <div className="overview-panel-head">
          <div>
            <p className="section-title">Relationship map</p>
            <h2>Graphical interface to your data</h2>
          </div>
          <p className="overview-copy">
            Click a node to jump into that resource. Drag from a source column on the right side of
            one table card to a target column on the left side of another table card to stage a
            relationship in the schema draft.
          </p>
        </div>
        <RelationMap
          overview={overviewQuery.data ?? null}
          schemaEdges={schemaEdges}
          selectedResourceName={selectedResource?.name ?? null}
          onSelectResource={selectResource}
          onCreateRelationship={handleRelationshipCreate}
          connectable={parsedSchemaDraft.document !== null}
          loading={overviewQuery.isLoading}
        />
      </section>

      <section className="overview-workspace">
        <aside className={`workspace-sidebar shell-card ${mobilePanel === 'data' ? 'mobile-active' : ''}`}>
          <div className="overview-panel-head sidebar-head">
            <div>
              <p className="section-title">Resources</p>
              <h2>Browse the source</h2>
            </div>
            <span className="overview-inline-badge">{resources.length}</span>
          </div>
          <label className="sidebar-search-label" htmlFor="resource-search">
            Filter resources
          </label>
          <input
            id="resource-search"
            className="overview-input"
            value={sidebarSearch}
            onChange={(event) => setSidebarSearch(event.target.value)}
            placeholder="Search by name or field"
          />
          <div className="resource-list">
            {filteredResources.map((resource) => (
              <button
                key={resource.name}
                type="button"
                className={`resource-list-item ${resource.name === selectedResource?.name ? 'is-selected' : ''}`}
                onClick={() => selectResource(resource.name)}
              >
                <div className="resource-list-copy">
                  <strong>{resource.name}</strong>
                  <span>{resource.kind}</span>
                </div>
                <div className="resource-list-meta">
                  {resource.row_count !== null ? (
                    <span>{resource.row_count} rows</span>
                  ) : resource.key_count !== null ? (
                    <span>{resource.key_count} keys</span>
                  ) : (
                    <span>value</span>
                  )}
                </div>
              </button>
            ))}
            {!filteredResources.length && (
              <p className="overview-empty">No resources match the current search.</p>
            )}
          </div>
        </aside>

        <div className={`workspace-main shell-card ${mobilePanel === 'data' ? 'mobile-active' : ''}`}>
          <div className="overview-panel-head">
            <div>
              <p className="section-title">Data explorer</p>
              <h2>{selectedResource?.name ?? 'Choose a resource'}</h2>
            </div>
            <div className="view-toggle" role="tablist" aria-label="Explorer view">
              <button
                type="button"
                className={urlState.view === 'explore' ? 'is-active' : ''}
                onClick={() => commitUrlState({ ...urlState, view: 'explore' })}
              >
                Explore
              </button>
              <button
                type="button"
                className={urlState.view === 'raw' ? 'is-active' : ''}
                onClick={() => commitUrlState({ ...urlState, view: 'raw' })}
              >
                Raw JSON
              </button>
            </div>
          </div>
          <DataExplorerPanel
            resource={selectedResource}
            response={resourceQuery.data}
            error={resourceQuery.error instanceof Error ? resourceQuery.error : null}
            isLoading={resourceQuery.isLoading}
            state={urlState}
            selectedRow={selectedRow}
            onStateChange={updateTableState}
            onRowSelect={(row) => {
              setSelectedRow(row);
              setMobilePanel('details');
            }}
            rawMode={urlState.view === 'raw'}
          />
        </div>

        <aside className={`workspace-details shell-card ${mobilePanel === 'details' ? 'mobile-active' : ''}`}>
          <div className="overview-panel-head">
            <div>
              <p className="section-title">Request panel</p>
              <h2>Shareable state</h2>
            </div>
            <button type="button" className="overview-secondary-button" onClick={() => void copyText(requestPath)}>
              Copy request URL
            </button>
          </div>
          <code className="request-path">{requestPath}</code>
          {copyStatus && <p className="copy-status">{copyStatus}</p>}
          {resourceQuery.data && (
            <div className="request-status-row">
              <span className={`status-pill ${resourceQuery.data.status >= 400 ? 'is-error' : ''}`}>
                {resourceQuery.data.status} {resourceQuery.data.statusText}
              </span>
              {isPaginatedResponse(resourceQuery.data.parsed) && (
                <span className="overview-inline-badge">
                  Page {resourceQuery.data.parsed.page} of {resourceQuery.data.parsed.pages}
                </span>
              )}
            </div>
          )}

          <section>
            <h3>Schema editor</h3>
            <p className="overview-copy">
              Edit the declared `schema.json` overlay directly and save it through `PUT /schema`.
            </p>
            <textarea
              className="schema-editor"
              value={schemaDraft}
              onChange={(event) => {
                updateSchemaDraft(event.target.value);
              }}
              spellCheck={false}
            />
            <div className="schema-editor-actions">
              <button
                type="button"
                className="overview-secondary-button"
                onClick={reloadSchemaDraft}
              >
                Reload schema
              </button>
              <button
                type="button"
                className="overview-secondary-button"
                onClick={() => saveSchemaMutation.mutate(schemaDraft)}
                disabled={saveSchemaMutation.isPending}
              >
                {saveSchemaMutation.isPending ? 'Saving…' : 'Save schema'}
              </button>
            </div>
            {parsedSchemaDraft.error && <p className="copy-status">{parsedSchemaDraft.error}</p>}
            {schemaStatus && <p className="copy-status">{schemaStatus}</p>}
          </section>

          {selectedResource?.kind === 'table' && selectedRow ? (
            <div className="details-stack">
              <section>
                <h3>Selected row</h3>
                <p className="overview-copy">
                  {selectedResource.primary_key && selectedRow[selectedResource.primary_key] !== undefined ? (
                    <>
                      Item route:{' '}
                      <code className="overview-inline-code">
                        /{selectedResource.name}/{String(selectedRow[selectedResource.primary_key])}
                      </code>
                    </>
                  ) : (
                    'Select a row to inspect the raw JSON payload and relation links.'
                  )}
                </p>
                <pre className="json-viewer">{formatJson(selectedRow)}</pre>
              </section>

              <section>
                <h3>Relation drill-down</h3>
                <div className="relation-link-list">
                  {selectedOutgoingLinks.length ? (
                    selectedOutgoingLinks.map(({ relation, value }) => (
                      <button
                        key={`${relation.source_table}:${relation.source_column}`}
                        type="button"
                        className="relation-link-button"
                        onClick={() =>
                          selectResource(relation.target_table, [
                            {
                              id: `${relation.target_column}:eq:drilldown`,
                              field: relation.target_column,
                              operator: 'eq',
                              value: value ?? ''
                            }
                          ])
                        }
                      >
                        <strong>{relation.target_table}</strong>
                        <span>
                          {relation.target_column} = {value}
                        </span>
                      </button>
                    ))
                  ) : (
                    <p className="overview-empty">No outgoing foreign-key links are available for the selected row.</p>
                  )}
                </div>
              </section>
            </div>
          ) : (
            <section>
              <h3>Resource snapshot</h3>
              <p className="overview-copy">
                {selectedResource
                  ? truncate(summarizeValue(resourceQuery.data?.parsed ?? selectedResource.row_samples[0] ?? null), 160)
                  : 'Choose a resource to inspect its current response payload.'}
              </p>
              {selectedResource && selectedResource.row_samples.length > 0 && (
                <pre className="json-viewer">{formatJson(selectedResource.row_samples[0])}</pre>
              )}
            </section>
          )}
        </aside>
      </section>
    </div>
  );

  function commitState(nextState: OverviewUrlState) {
    commitUrlState(nextState);
  }
}

interface RelationNodeData extends Record<string, unknown> {
  resource: ResourceOverview;
  selected: boolean;
}

type RelationFlowNode = Node<RelationNodeData, 'schemaTable'>;

const RELATION_NODE_TYPES = {
  schemaTable: RelationNodeCard
};

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
              <Handle
                type="target"
                position={Position.Left}
                id={buildSchemaHandleId('target', column.name)}
                className="graph-column-handle is-target"
              />
              <div className="graph-column-copy">
                <span className="graph-column-name">{column.name}</span>
                <span className="graph-column-meta">
                  {column.column_type}
                  {column.is_primary_key ? ' · pk' : ''}
                  {column.relation ? ' · fk' : ''}
                </span>
              </div>
              <Handle
                type="source"
                position={Position.Right}
                id={buildSchemaHandleId('source', column.name)}
                className="graph-column-handle is-source"
              />
            </div>
          ))}
        </div>
      ) : (
        <div className="graph-node-empty">No schema columns available for connection editing.</div>
      )}
    </div>
  );
}

function SummaryCard({ label, value, copy }: { label: string; value: string; copy: string }) {
  return (
    <article className="summary-card">
      <span className="section-title">{label}</span>
      <strong>{value}</strong>
      <p>{copy}</p>
    </article>
  );
}

function RelationMap({
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
    return <p className="overview-empty">Loading overview graph...</p>;
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
        selected: resource.name === selectedResourceName
      },
      selectable: false
    };
  });

  const edges: Edge[] = schemaEdges.map((edge) => ({
    id: `${edge.source_table}:${edge.source_column}:${edge.target_table}:${edge.target_column}`,
    source: edge.source_table,
    target: edge.target_table,
    sourceHandle: buildSchemaHandleId('source', edge.source_column),
    targetHandle: buildSchemaHandleId('target', edge.target_column),
    label: `${edge.source_column} -> ${edge.target_column}`,
    animated: edge.source_table === selectedResourceName || edge.target_table === selectedResourceName,
    style:
      edge.source_table === selectedResourceName || edge.target_table === selectedResourceName
        ? { stroke: '#0f766e', strokeWidth: 2.4 }
        : { stroke: 'rgba(94, 109, 104, 0.42)', strokeWidth: 1.8 },
    labelStyle: { fill: '#39554d', fontWeight: 600 }
  }));

  return (
    <div className="relation-map-shell">
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

export function DataExplorerPanel({
  resource,
  response,
  error,
  isLoading,
  state,
  selectedRow,
  onStateChange,
  onRowSelect,
  rawMode
}: {
  resource: ResourceOverview | null;
  response: ResourceResponse | undefined;
  error: Error | null;
  isLoading: boolean;
  state: OverviewUrlState;
  selectedRow: Record<string, unknown> | null;
  onStateChange: (updater: (state: OverviewUrlState) => OverviewUrlState) => void;
  onRowSelect: (row: Record<string, unknown> | null) => void;
  rawMode: boolean;
}) {
  const rows = resource?.kind === 'table' ? getTableRows(response?.parsed) : [];
  const columnNames = resource?.kind === 'table' ? getColumnNames(resource, rows) : [];
  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({});
  const columnSignature = columnNames.join(',');

  useEffect(() => {
    if (!resource || resource.kind !== 'table') {
      setColumnVisibility((current) => (Object.keys(current).length === 0 ? current : {}));
      return;
    }
    setColumnVisibility((current) => {
      const nextVisibility: VisibilityState = {};
      for (const name of columnNames) {
        nextVisibility[name] = current[name] ?? true;
      }
      if (shallowVisibilityEquals(current, nextVisibility)) {
        return current;
      }
      return nextVisibility;
    });
  }, [columnSignature, resource?.kind, resource?.name]);

  const relationColumns = resource?.columns.filter((column) => Boolean(column.relation)) ?? [];
  const fieldOptions = columnNames.length > 0 ? columnNames : ['id'];
  const sortingState = state.sorting.map((sort) => ({ id: sort.id, desc: sort.desc }));
  const primaryKey = resource?.primary_key ?? null;
  const selectedPrimaryValue = primaryKey && selectedRow ? selectedRow[primaryKey] : undefined;

  const columns: ColumnDef<Record<string, unknown>>[] = columnNames.map((columnName) => {
    const resourceColumn = resource?.columns.find((column) => column.name === columnName);
    return {
      id: columnName,
      accessorFn: (row) => row[columnName],
      header: () => null,
      cell: ({ row }) => (
        <div className={`cell-content ${resourceColumn?.relation ? 'is-relation' : ''}`}>
          {renderCellValue(row.original[columnName])}
        </div>
      ),
      enableHiding: true
    };
  });

  const table = useReactTable({
    data: rows,
    columns,
    getCoreRowModel: getCoreRowModel(),
    manualSorting: true,
    manualPagination: true,
    pageCount: isPaginatedResponse(response?.parsed) ? response?.parsed.pages : 1,
    state: {
      columnVisibility,
      sorting: sortingState
    },
    onColumnVisibilityChange: setColumnVisibility
  });

  if (!resource) {
    return <p className="overview-empty">Choose a resource to start exploring the data.</p>;
  }

  if (isLoading) {
    return <p className="overview-empty">Loading {resource.name}...</p>;
  }

  if (error) {
    return (
      <div className="error-state">
        <p className="section-title">Request failed</p>
        <pre className="json-viewer">{error.message}</pre>
      </div>
    );
  }

  if (rawMode) {
    return <pre className="json-viewer">{formatJson(response?.parsed ?? null)}</pre>;
  }

  if (resource.kind !== 'table') {
    return (
      <div className="non-table-panel" data-testid="non-table-view">
        <p className="overview-copy">
          This resource is not an array, so the explorer stays in JSON mode instead of rendering the
          table controls.
        </p>
        <pre className="json-viewer">{formatJson(response?.parsed ?? null)}</pre>
      </div>
    );
  }

  return (
    <div className="data-explorer-stack">
      <div className="control-bar">
        <FilterBuilder
          fields={fieldOptions}
          filters={state.filters}
          onAddFilter={() =>
            onStateChange((current) => ({
              ...current,
              page: 1,
              filters: [...current.filters, createFilter(fieldOptions[0] ?? 'id')]
            }))
          }
          onChangeFilter={(filterId, patch) =>
            onStateChange((current) => ({
              ...current,
              page: 1,
              filters: current.filters.map((filter) =>
                filter.id === filterId ? { ...filter, ...patch } : filter
              )
            }))
          }
          onRemoveFilter={(filterId) =>
            onStateChange((current) => ({
              ...current,
              page: 1,
              filters: current.filters.filter((filter) => filter.id !== filterId)
            }))
          }
        />

        <div className="secondary-controls">
          <label>
            Page size
            <select
              className="overview-select"
              value={state.perPage}
              onChange={(event) =>
                onStateChange((current) => ({
                  ...current,
                  page: 1,
                  perPage: Number(event.target.value)
                }))
              }
            >
              {PAGE_SIZE_OPTIONS.map((size) => (
                <option key={size} value={size}>
                  {size}
                </option>
              ))}
            </select>
          </label>

          {relationColumns.length > 0 && (
            <fieldset className="embed-fieldset">
              <legend>Embed relations</legend>
              {relationColumns.map((column) => (
                <label key={column.name} className="embed-option">
                  <input
                    type="checkbox"
                    checked={state.embeds.includes(column.name)}
                    onChange={(event) =>
                      onStateChange((current) => ({
                        ...current,
                        page: 1,
                        embeds: event.target.checked
                          ? [...current.embeds, column.name]
                          : current.embeds.filter((entry) => entry !== column.name)
                      }))
                    }
                  />
                  <span>
                    {column.name}
                    {column.relation ? ` -> ${column.relation}` : ''}
                  </span>
                </label>
              ))}
            </fieldset>
          )}
        </div>
      </div>

      <details className="column-picker">
        <summary>Visible columns</summary>
        <div className="column-picker-grid">
          {table.getAllLeafColumns().map((column) => (
            <label key={column.id} className="column-toggle">
              <input
                type="checkbox"
                checked={column.getIsVisible()}
                onChange={column.getToggleVisibilityHandler()}
              />
              <span>{column.id}</span>
            </label>
          ))}
        </div>
      </details>

      <div className="table-shell">
        <table className="data-table">
          <thead>
            <tr>
              {table.getHeaderGroups()[0]?.headers.map((header) => {
                const sortEntry = state.sorting.find((entry) => entry.id === header.column.id);
                return (
                  <th key={header.id}>
                    <button
                      type="button"
                      className="column-header-button"
                      onClick={(event: MouseEvent<HTMLButtonElement>) =>
                        onStateChange((current) => ({
                          ...current,
                          page: 1,
                          sorting: nextSorting(current.sorting, header.column.id, event.shiftKey)
                        }))
                      }
                    >
                      <span>{header.column.id}</span>
                      <span className="sort-indicator">
                        {sortEntry ? (sortEntry.desc ? 'desc' : 'asc') : 'sort'}
                      </span>
                    </button>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {table.getRowModel().rows.map((row) => {
              const isSelected =
                primaryKey && selectedPrimaryValue !== undefined
                  ? row.original[primaryKey] === selectedPrimaryValue
                  : selectedRow === row.original;
              return (
                <tr
                  key={row.id}
                  className={isSelected ? 'is-selected' : ''}
                  onClick={() => onRowSelect(row.original)}
                >
                  {row.getVisibleCells().map((cell) => (
                    <td key={cell.id}>{flexRender(cell.column.columnDef.cell, cell.getContext())}</td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div className="pagination-row">
        <button
          type="button"
          className="overview-secondary-button"
          disabled={state.page <= 1}
          onClick={() => onStateChange((current) => ({ ...current, page: Math.max(1, current.page - 1) }))}
        >
          Previous
        </button>
        <span className="overview-inline-badge">
          {isPaginatedResponse(response?.parsed)
            ? `Page ${response?.parsed.page} of ${response?.parsed.pages}`
            : `Page ${state.page}`}
        </span>
        <button
          type="button"
          className="overview-secondary-button"
          disabled={isPaginatedResponse(response?.parsed) ? response.parsed.next === null : rows.length < state.perPage}
          onClick={() => onStateChange((current) => ({ ...current, page: current.page + 1 }))}
        >
          Next
        </button>
      </div>
    </div>
  );
}

function FilterBuilder({
  fields,
  filters,
  onAddFilter,
  onChangeFilter,
  onRemoveFilter
}: {
  fields: string[];
  filters: FilterDescriptor[];
  onAddFilter: () => void;
  onChangeFilter: (filterId: string, patch: Partial<FilterDescriptor>) => void;
  onRemoveFilter: (filterId: string) => void;
}) {
  return (
    <div className="filter-builder">
      <div className="filter-builder-head">
        <div>
          <span className="section-title">Filters</span>
          <p className="overview-copy">Server-backed operators map directly to the REST query string.</p>
        </div>
        <button type="button" className="overview-secondary-button" onClick={onAddFilter}>
          Add filter
        </button>
      </div>
      <div className="filter-list">
        {filters.map((filter) => (
          <div key={filter.id} className="filter-row">
            <select
              className="overview-select"
              value={filter.field}
              onChange={(event) => onChangeFilter(filter.id, { field: event.target.value })}
            >
              {fields.map((field) => (
                <option key={field} value={field}>
                  {field}
                </option>
              ))}
            </select>
            <select
              className="overview-select"
              value={filter.operator}
              onChange={(event) =>
                onChangeFilter(filter.id, {
                  operator: event.target.value as FilterOperator,
                  value:
                    event.target.value === 'isNull' || event.target.value === 'isNotNull'
                      ? ''
                      : filter.value
                })
              }
            >
              {FILTER_OPERATORS.map((operator) => (
                <option key={operator} value={operator}>
                  {operator}
                </option>
              ))}
            </select>
            {filter.operator === 'isNull' || filter.operator === 'isNotNull' ? (
              <span className="filter-implicit-value">No value</span>
            ) : (
              <input
                className="overview-input"
                value={filter.value}
                onChange={(event) => onChangeFilter(filter.id, { value: event.target.value })}
                placeholder="value"
              />
            )}
            <button
              type="button"
              className="overview-icon-button"
              onClick={() => onRemoveFilter(filter.id)}
              aria-label={`Remove filter on ${filter.field}`}
            >
              Remove
            </button>
          </div>
        ))}
        {filters.length === 0 && <p className="overview-empty">No filters yet. Add one to narrow the result set.</p>}
      </div>
    </div>
  );
}

function renderCellValue(value: unknown) {
  if (value === null || value === undefined) {
    return <span className="cell-muted">null</span>;
  }
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return <span>{String(value)}</span>;
  }
  if (Array.isArray(value)) {
    return (
      <details className="cell-disclosure">
        <summary>{`Array(${value.length})`}</summary>
        <pre className="json-viewer compact">{formatJson(value)}</pre>
      </details>
    );
  }
  if (isRecord(value)) {
    return (
      <details className="cell-disclosure">
        <summary>{summarizeValue(value)}</summary>
        <pre className="json-viewer compact">{formatJson(value)}</pre>
      </details>
    );
  }
  return <span>{String(value)}</span>;
}

function shallowVisibilityEquals(left: VisibilityState, right: VisibilityState): boolean {
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }
  return leftKeys.every((key) => left[key] === right[key]);
}
