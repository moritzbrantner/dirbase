import { QueryClient, QueryClientProvider, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
  type Updater,
  type VisibilityState
} from '@tanstack/react-table';
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  type Edge,
  type Node
} from '@xyflow/react';
import { startTransition, useDeferredValue, useEffect, useState, type MouseEvent, type ReactNode } from 'react';

import {
  fetchOverview,
  fetchResource,
  fetchSchema,
  inferSchemaDocument,
  mutateResource,
  saveSchemaDocument
} from './api';
import {
  coerceIncomingRelationValue,
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
  DEFAULT_PREFERENCES,
  FILTER_OPERATOR_LABELS,
  buildMutationPlan,
  buildQuerySummaryChips,
  getVisibleMutationActions,
  loadOverviewPreferences,
  saveOverviewPreferences,
  summarizeSchemaDiff
} from './overviewUtils';
import type {
  FilterDescriptor,
  FilterOperator,
  InspectorTab,
  LiveUpdateStatus,
  MutationPlan,
  OverviewPageData,
  OverviewPreferences,
  OverviewRelation,
  OverviewUiState,
  OverviewUrlState,
  QuerySummaryChip,
  ResourceOverview,
  ResourceResponse,
  ServerCapabilities
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

interface ToastMessage {
  id: number;
  tone: 'info' | 'success' | 'error';
  message: string;
}

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
  const client = useQueryClient();
  const [urlState, setUrlState] = useState<OverviewUrlState>(() => parseOverviewState(window.location.search));
  const [sidebarSearch, setSidebarSearch] = useState('');
  const deferredSidebarSearch = useDeferredValue(sidebarSearch);
  const [preferences, setPreferences] = useState<OverviewPreferences>(() =>
    loadOverviewPreferences(window.localStorage)
  );
  const [uiState, setUiState] = useState<OverviewUiState>(() => ({
    selectedResource: urlState.resource,
    selectedRow: null,
    inspectorTab: preferences.lastInspectorTab,
    liveUpdates: 'connecting',
    mutationDialog: { open: false, mode: null },
    readonly: false
  }));
  const [schemaDraft, setSchemaDraft] = useState('{}');
  const [loadedSchemaText, setLoadedSchemaText] = useState('{}');
  const [schemaStatus, setSchemaStatus] = useState<string | null>(null);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const [eventStreamKey, setEventStreamKey] = useState(0);

  const overviewQuery = useQuery({
    queryKey: ['overview', overviewEndpoint],
    queryFn: () => fetchOverview(overviewEndpoint)
  });
  const schemaQuery = useQuery({
    queryKey: ['schema'],
    queryFn: () => fetchSchema()
  });

  const resources = overviewQuery.data?.resources ?? [];
  const selectedResource =
    resources.find((resource) => resource.name === urlState.resource) ?? resources[0] ?? null;
  const serverCapabilities = overviewQuery.data?.server_capabilities ?? null;
  const requestPath = buildResourceRequestPath(selectedResource, urlState);
  const requestUrl = `${window.location.origin}${requestPath}`;

  const resourceQuery = useQuery({
    queryKey: ['resource', requestPath],
    queryFn: () => fetchResource(requestPath),
    enabled: Boolean(selectedResource)
  });

  const saveSchemaMutation = useMutation({
    mutationFn: saveSchemaDocument,
    onSuccess: async () => {
      setSchemaStatus('Schema saved.');
      pushToast('Schema saved.', 'success');
      await client.invalidateQueries({ queryKey: ['schema'] });
      await client.invalidateQueries({ queryKey: ['overview'] });
      await client.invalidateQueries({ queryKey: ['resource'] });
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : 'Schema save failed.';
      setSchemaStatus(message);
      pushToast(message, 'error');
    }
  });
  const inferSchemaMutation = useMutation({
    mutationFn: inferSchemaDocument,
    onSuccess: async (result) => {
      setSchemaStatus(`Schema inferred${result.path ? ` to ${result.path}` : ''}.`);
      pushToast(`Schema inferred${result.path ? `: ${result.path}` : '.'}`, 'success');
      await client.invalidateQueries({ queryKey: ['schema'] });
      await client.invalidateQueries({ queryKey: ['overview'] });
      await client.invalidateQueries({ queryKey: ['resource'] });
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : 'Schema infer failed.';
      setSchemaStatus(message);
      pushToast(message, 'error');
    }
  });

  const selectedRow = uiState.selectedRow;
  const tableRows = selectedResource?.kind === 'table' ? getTableRows(resourceQuery.data?.parsed) : [];
  const querySummaryChips = buildQuerySummaryChips({
    filters: urlState.filters,
    sorting: urlState.sorting,
    embeds: urlState.embeds
  });
  const mutationActions = getVisibleMutationActions(selectedResource, serverCapabilities, selectedRow);
  const outgoingRelationLinks =
    selectedResource && selectedRow
      ? selectedResource.outgoing_relations
          .map((relation) => ({
            relation,
            value: coerceRelationValue(selectedRow, relation)
          }))
          .filter((entry) => entry.value !== null)
      : [];
  const incomingRelationLinks =
    selectedResource && selectedRow
      ? selectedResource.incoming_relations
          .map((relation) => ({
            relation,
            value: coerceIncomingRelationValue(selectedRow, relation)
          }))
          .filter((entry) => entry.value !== null)
      : [];
  const columnVisibility =
    selectedResource?.name ? preferences.columnVisibility[selectedResource.name] ?? {} : {};
  const schemaDiffSummary =
    schemaDraft.trim() !== loadedSchemaText.trim() ? summarizeSchemaDiff(loadedSchemaText, schemaDraft) : [];
  const schemaValidationError = getJsonValidationError(schemaDraft);

  useEffect(() => {
    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, []);

  useEffect(() => {
    saveOverviewPreferences(window.localStorage, preferences);
  }, [preferences]);

  useEffect(() => {
    if (!schemaQuery.data) {
      return;
    }
    const nextText = formatJson(schemaQuery.data);
    setLoadedSchemaText(nextText);
    setSchemaDraft(nextText);
  }, [schemaQuery.data]);

  useEffect(() => {
    if (!selectedResource) {
      return;
    }
    if (urlState.resource === selectedResource.name) {
      return;
    }
    commitUrlState(resetTableState(selectedResource.name, urlState.view));
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
      commitUrlState(resetTableState(selectedResource.name, urlState.view));
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

  useEffect(() => {
    const readonly = overviewQuery.data?.server_capabilities.readonly ?? false;
    setUiState((current) => {
      const nextResource = selectedResource?.name ?? null;
      const resourceChanged = current.selectedResource !== nextResource;
      const nextInspectorTab =
        resourceChanged || (!current.selectedRow && current.inspectorTab === 'selection')
          ? 'request'
          : current.inspectorTab;
      if (
        current.selectedResource === nextResource &&
        current.readonly === readonly &&
        !resourceChanged &&
        current.inspectorTab === nextInspectorTab
      ) {
        return current;
      }
      return {
        ...current,
        selectedResource: nextResource,
        selectedRow: resourceChanged ? null : current.selectedRow,
        inspectorTab: nextInspectorTab,
        readonly
      };
    });
  }, [overviewQuery.data?.server_capabilities.readonly, selectedResource?.name]);

  useEffect(() => {
    if (selectedResource?.kind !== 'table') {
      setUiState((current) => {
        if (!current.selectedRow) {
          return current;
        }
        return {
          ...current,
          selectedRow: null,
          inspectorTab: current.inspectorTab === 'selection' ? 'request' : current.inspectorTab
        };
      });
      return;
    }

    if (!uiState.selectedRow || !selectedResource.primary_key) {
      return;
    }

    const selectedValue = uiState.selectedRow[selectedResource.primary_key];
    const nextRow = tableRows.find((row) => row[selectedResource.primary_key as string] === selectedValue) ?? null;
    if (nextRow === uiState.selectedRow) {
      return;
    }
    setUiState((current) => ({
      ...current,
      selectedRow: nextRow,
      inspectorTab: nextRow ? current.inspectorTab : current.inspectorTab === 'selection' ? 'request' : current.inspectorTab
    }));
  }, [selectedResource, tableRows, uiState.selectedRow]);

  useEffect(() => {
    let active = true;
    let source: EventSource | null = null;
    let retries = 0;
    let reconnectTimer: number | null = null;

    function connect() {
      if (!active) {
        return;
      }
      setUiState((current) => ({
        ...current,
        liveUpdates: retries === 0 ? 'connecting' : 'reconnecting'
      }));

      source = new EventSource('/events');
      source.onopen = () => {
        const wasReconnecting = retries > 0;
        retries = 0;
        setUiState((current) => ({ ...current, liveUpdates: 'live' }));
        if (wasReconnecting) {
          pushToast('Reconnected to live updates.', 'success');
        }
      };

      const handleServerEvent = (label: string) => {
        void client.invalidateQueries({ queryKey: ['overview'] });
        void client.invalidateQueries({ queryKey: ['resource'] });
        void client.invalidateQueries({ queryKey: ['schema'] });
        pushToast(label, 'info');
      };

      source.addEventListener('overview_changed', () => handleServerEvent('Overview changed. Refreshed.'));
      source.addEventListener('resource_changed', () => handleServerEvent('Data changed. Refreshed.'));
      source.addEventListener('schema_changed', () => handleServerEvent('Schema changed. Refreshed.'));
      source.onerror = () => {
        source?.close();
        if (!active) {
          return;
        }
        retries += 1;
        if (retries >= 3) {
          setUiState((current) => ({ ...current, liveUpdates: 'paused' }));
          pushToast('Live updates paused.', 'error');
          return;
        }
        setUiState((current) => ({ ...current, liveUpdates: 'reconnecting' }));
        reconnectTimer = window.setTimeout(connect, retries * 1_500);
      };
    }

    connect();
    return () => {
      active = false;
      source?.close();
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer);
      }
    };
  }, [client, eventStreamKey]);

  const groupedResources = groupResources(
    resources.filter((resource) => {
      if (!deferredSidebarSearch.trim()) {
        return true;
      }
      const needle = deferredSidebarSearch.trim().toLowerCase();
      return (
        resource.name.toLowerCase().includes(needle) ||
        resource.field_names.some((field) => field.toLowerCase().includes(needle))
      );
    })
  );

  return (
    <div className="overview-app-shell">
      <section className="overview-summary-grid">
        <SummaryCard
          label="Resources"
          value={overviewQuery.isLoading ? null : String(overviewQuery.data?.stats.resource_count ?? 0)}
          copy="Tables, objects, and scalar resources exposed by the server."
        />
        <SummaryCard
          label="Relations"
          value={overviewQuery.isLoading ? null : String(overviewQuery.data?.stats.relation_count ?? 0)}
          copy="Foreign-key links that drive drill-down and embeds."
        />
        <SummaryCard
          label="Rows"
          value={overviewQuery.isLoading ? null : String(overviewQuery.data?.stats.total_rows ?? 0)}
          copy="Approximate array rows across table resources."
        />
        <SummaryCard
          label="Source"
          value={overviewQuery.isLoading ? null : overviewQuery.data?.data_source_kind ?? 'unknown'}
          copy={overviewQuery.data?.source_rule ?? 'Loading source metadata...'}
        />
      </section>

      <section className="overview-status-card shell-card">
        <div className="overview-status-group">
          <span className={`status-pill is-live-${uiState.liveUpdates}`}>
            Live updates: {renderLiveUpdateLabel(uiState.liveUpdates)}
          </span>
          {uiState.readonly && <span className="status-pill is-warn">Read-only mode</span>}
          {overviewQuery.data?.schema_enabled && <span className="status-pill">Schema loaded</span>}
        </div>
        <div className="overview-status-group">
          <code className="overview-source-line">{overviewQuery.data?.source_label ?? 'Loading source...'}</code>
          {uiState.liveUpdates === 'paused' && (
            <button
              type="button"
              className="overview-secondary-button"
              onClick={() => setEventStreamKey((current) => current + 1)}
            >
              Retry live updates
            </button>
          )}
        </div>
      </section>

      <section
        className={`overview-panel shell-card relation-map-panel ${
          preferences.mobileSurface === 'map' ? 'mobile-drawer-open' : ''
        }`}
      >
        <div className="overview-panel-head">
          <div>
            <p className="section-title">Relation map</p>
            <h2>Drill through resource links</h2>
          </div>
          <p className="overview-copy">
            Click a node to switch resources. The map reflects the same schema metadata that powers
            embeds and relation drill-down.
          </p>
        </div>
        <RelationMap
          overview={overviewQuery.data ?? null}
          selectedResourceName={selectedResource?.name ?? null}
          onSelectResource={selectResource}
          loading={overviewQuery.isLoading}
        />
      </section>

      <section className="overview-workspace">
        <ResourceSidebar
          groupedResources={groupedResources}
          loading={overviewQuery.isLoading}
          search={sidebarSearch}
          selectedResourceName={selectedResource?.name ?? null}
          searchNeedle={deferredSidebarSearch}
          mobileOpen={preferences.mobileSurface === 'resources'}
          onSearchChange={setSidebarSearch}
          onSelectResource={selectResource}
        />

        <main className="workspace-main shell-card">
          <ExplorerHeader
            resource={selectedResource}
            selectedRow={uiState.selectedRow}
            readonly={uiState.readonly}
            view={urlState.view}
            actions={mutationActions}
            onChangeView={(view) => commitUrlState({ ...urlState, view })}
            onOpenCreate={() => openMutationDialog('create')}
            onOpenEdit={() =>
              openMutationDialog(selectedResource?.kind === 'object' ? 'editObject' : 'edit')
            }
            onOpenDelete={() => openMutationDialog('delete')}
          />

          <QuerySummaryBar
            chips={querySummaryChips}
            hasState={urlState.filters.length > 0 || urlState.sorting.length > 0 || urlState.embeds.length > 0}
            onClear={() =>
              updateTableState((current) => ({
                ...current,
                page: 1,
                filters: [],
                sorting: [],
                embeds: []
              }))
            }
            onRemoveChip={(chip) => {
              if (chip.kind === 'filter') {
                updateTableState((current) => ({
                  ...current,
                  page: 1,
                  filters: current.filters.filter((filter) => filter.id !== chip.id)
                }));
                return;
              }
              if (chip.kind === 'sort') {
                const columnId = chip.id.replace('sort:', '');
                updateTableState((current) => ({
                  ...current,
                  page: 1,
                  sorting: current.sorting.filter((sort) => sort.id !== columnId)
                }));
                return;
              }
              const embed = chip.id.replace('embed:', '');
              updateTableState((current) => ({
                ...current,
                page: 1,
                embeds: current.embeds.filter((entry) => entry !== embed)
              }));
            }}
          />

          <DataExplorerPanel
            resource={selectedResource}
            response={resourceQuery.data}
            error={resourceQuery.error instanceof Error ? resourceQuery.error : null}
            isLoading={resourceQuery.isLoading}
            state={urlState}
            selectedRow={uiState.selectedRow}
            rawMode={urlState.view === 'raw'}
            columnVisibility={columnVisibility}
            onColumnVisibilityChange={handleColumnVisibilityChange}
            onStateChange={updateTableState}
            onRowSelect={(row) => {
              setUiState((current) => ({
                ...current,
                selectedRow: row,
                inspectorTab: 'selection'
              }));
              setPreferences((current) => ({ ...current, mobileSurface: 'inspector' }));
            }}
          />
        </main>

        <InspectorPanel
          resource={selectedResource}
          response={resourceQuery.data}
          schemaDraft={schemaDraft}
          schemaStatus={schemaStatus}
          schemaDiffSummary={schemaDiffSummary}
          schemaValidationError={schemaValidationError}
          selectedRow={uiState.selectedRow}
          selectedTab={uiState.inspectorTab}
          outgoingRelations={outgoingRelationLinks}
          incomingRelations={incomingRelationLinks}
          readonly={uiState.readonly}
          mobileOpen={preferences.mobileSurface === 'inspector'}
          schemaBusy={saveSchemaMutation.isPending || inferSchemaMutation.isPending}
          canSaveSchema={Boolean(serverCapabilities?.schema_write)}
          canInferSchema={Boolean(serverCapabilities?.schema_infer)}
          requestPath={requestPath}
          requestUrl={requestUrl}
          onTabChange={setInspectorTab}
          onSchemaDraftChange={(value) => {
            setSchemaDraft(value);
            setSchemaStatus(null);
          }}
          onCopy={copyText}
          onOpenRequest={() => window.open(requestUrl, '_blank', 'noopener,noreferrer')}
          onReloadSchema={() => void client.invalidateQueries({ queryKey: ['schema'] })}
          onSaveSchema={() => handleSaveSchema()}
          onInferSchema={() => inferSchemaMutation.mutate()}
          onDrilldownOutgoing={({ relation, value }) =>
            selectResource(relation.target_table, [buildDrilldownFilter(relation.target_column, value)])
          }
          onDrilldownIncoming={({ relation, value }) =>
            selectResource(relation.source_table, [buildDrilldownFilter(relation.source_column, value)])
          }
        />
      </section>

      <div className="mobile-sticky-actions">
        <button
          type="button"
          className={preferences.mobileSurface === 'resources' ? 'is-active' : ''}
          onClick={() => toggleMobileSurface('resources')}
        >
          Resources
        </button>
        <button
          type="button"
          className={preferences.mobileSurface === 'map' ? 'is-active' : ''}
          onClick={() => toggleMobileSurface('map')}
        >
          Map
        </button>
        <button
          type="button"
          className={preferences.mobileSurface === 'inspector' ? 'is-active' : ''}
          onClick={() => toggleMobileSurface('inspector')}
        >
          Inspector
        </button>
      </div>

      <MutationDialog
        open={uiState.mutationDialog.open}
        mode={uiState.mutationDialog.mode}
        resource={selectedResource}
        selectedRow={uiState.selectedRow}
        objectValue={selectedResource?.kind === 'object' ? resourceQuery.data?.parsed : null}
        onClose={() =>
          setUiState((current) => ({
            ...current,
            mutationDialog: { open: false, mode: null }
          }))
        }
        onSubmit={submitMutationPlan}
      />

      <ToastViewport toasts={toasts} />
    </div>
  );

  function handlePopState() {
    startTransition(() => {
      setUrlState(parseOverviewState(window.location.search));
    });
  }

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
    setUiState((current) => ({
      ...current,
      selectedResource: resourceName,
      selectedRow: null,
      inspectorTab: 'request'
    }));
    setPreferences((current) => ({ ...current, mobileSurface: 'explorer' }));
  }

  function updateTableState(updater: (state: OverviewUrlState) => OverviewUrlState) {
    if (!selectedResource) {
      return;
    }
    commitUrlState(updater({ ...urlState, resource: selectedResource.name }));
  }

  function handleColumnVisibilityChange(resourceName: string, updater: Updater<VisibilityState>) {
    setPreferences((current) => {
      const previous = current.columnVisibility[resourceName] ?? {};
      const next =
        typeof updater === 'function'
          ? (updater as (state: VisibilityState) => VisibilityState)(previous)
          : updater;
      return {
        ...current,
        columnVisibility: {
          ...current.columnVisibility,
          [resourceName]: next
        }
      };
    });
  }

  function setInspectorTab(tab: InspectorTab) {
    setUiState((current) => ({ ...current, inspectorTab: tab }));
    setPreferences((current) => ({ ...current, lastInspectorTab: tab }));
  }

  function toggleMobileSurface(surface: OverviewPreferences['mobileSurface']) {
    setPreferences((current) => ({
      ...current,
      mobileSurface: current.mobileSurface === surface ? 'explorer' : surface
    }));
  }

  function openMutationDialog(mode: 'create' | 'edit' | 'delete' | 'editObject') {
    setUiState((current) => ({
      ...current,
      mutationDialog: { open: true, mode }
    }));
  }

  async function handleSaveSchema() {
    if (schemaValidationError) {
      setSchemaStatus(schemaValidationError);
      pushToast(schemaValidationError, 'error');
      return;
    }
    saveSchemaMutation.mutate(schemaDraft);
  }

  async function submitMutationPlan(plan: MutationPlan) {
    const result = await mutateResource({
      method: plan.method,
      path: plan.path,
      body: plan.body
    });

    await client.invalidateQueries({ queryKey: ['overview'] });
    await client.invalidateQueries({ queryKey: ['resource'] });
    await client.invalidateQueries({ queryKey: ['schema'] });

    if (plan.method === 'DELETE') {
      setUiState((current) => ({
        ...current,
        selectedRow: null,
        inspectorTab: current.inspectorTab === 'selection' ? 'request' : current.inspectorTab
      }));
    } else if (isRecord(result.parsed) && selectedResource?.kind === 'table') {
      const nextSelectedRow = result.parsed;
      setUiState((current) => ({
        ...current,
        selectedRow: nextSelectedRow,
        inspectorTab: 'selection'
      }));
    }

    pushToast(`${plan.method} ${plan.path}`, 'success');
  }

  async function copyText(value: string) {
    if (!navigator.clipboard) {
      pushToast('Clipboard is unavailable in this browser.', 'error');
      return;
    }
    await navigator.clipboard.writeText(value);
    pushToast('Copied to clipboard.', 'success');
  }

  function pushToast(message: string, tone: ToastMessage['tone']) {
    const id = Date.now() + Math.floor(Math.random() * 1000);
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 3_000);
  }
}

function SummaryCard({ label, value, copy }: { label: string; value: string | null; copy: string }) {
  return (
    <article className="summary-card">
      <span className="section-title">{label}</span>
      {value === null ? <div className="skeleton skeleton-title" /> : <strong>{value}</strong>}
      <p>{copy}</p>
    </article>
  );
}

function ResourceSidebar({
  groupedResources,
  loading,
  search,
  selectedResourceName,
  searchNeedle,
  mobileOpen,
  onSearchChange,
  onSelectResource
}: {
  groupedResources: Record<ResourceOverview['kind'], ResourceOverview[]>;
  loading: boolean;
  search: string;
  selectedResourceName: string | null;
  searchNeedle: string;
  mobileOpen: boolean;
  onSearchChange: (value: string) => void;
  onSelectResource: (resourceName: string) => void;
}) {
  return (
    <aside
      className={`workspace-sidebar shell-card ${mobileOpen ? 'mobile-drawer-open' : ''}`}
      data-testid="resource-sidebar"
    >
      <div className="overview-panel-head">
        <div>
          <p className="section-title">Resources</p>
          <h2>Browse and compare</h2>
        </div>
      </div>
      <p className="overview-copy">
        Resources are REST routes over JSON files or top-level JSON keys. Search by route name or field.
      </p>
      <label className="sidebar-search-label" htmlFor="resource-search">
        Search resources
      </label>
      <input
        id="resource-search"
        className="overview-input"
        value={search}
        onChange={(event) => onSearchChange(event.target.value)}
        placeholder="Search by resource or field"
      />

      {loading ? (
        <div className="resource-list">
          {Array.from({ length: 6 }).map((_, index) => (
            <div key={index} className="resource-list-item is-skeleton">
              <div className="skeleton skeleton-line" />
              <div className="skeleton skeleton-chip-row" />
            </div>
          ))}
        </div>
      ) : (
        <div className="resource-group-stack">
          {(['table', 'object', 'value'] as const).map((kind) =>
            groupedResources[kind].length > 0 ? (
              <section key={kind} className="resource-group">
                <div className="resource-group-head">
                  <span className="section-title">{kind}</span>
                  <span className="overview-inline-badge">{groupedResources[kind].length}</span>
                </div>
                <div className="resource-list">
                  {groupedResources[kind].map((resource) => (
                    <button
                      key={resource.name}
                      type="button"
                      className={`resource-list-item ${
                        resource.name === selectedResourceName ? 'is-selected' : ''
                      }`}
                      onClick={() => onSelectResource(resource.name)}
                    >
                      <div className="resource-list-copy">
                        <div className="resource-list-head">
                          <strong>
                            <HighlightText text={resource.name} needle={searchNeedle} />
                          </strong>
                          <span className="overview-kind-badge">{resource.kind}</span>
                        </div>
                        <div className="resource-list-meta-row">
                          <span>
                            {resource.row_count !== null
                              ? `${resource.row_count} rows`
                              : resource.key_count !== null
                                ? `${resource.key_count} keys`
                                : 'scalar value'}
                          </span>
                          {resource.primary_key && <span>PK: {resource.primary_key}</span>}
                          <span>{resource.outgoing_relations.length} outgoing links</span>
                        </div>
                        {resource.field_names.length > 0 && (
                          <div className="resource-field-list">
                            {resource.field_names.slice(0, 4).map((field) => (
                              <span key={field} className="resource-field-pill">
                                <HighlightText text={field} needle={searchNeedle} />
                              </span>
                            ))}
                          </div>
                        )}
                      </div>
                      <div className="resource-capability-row">
                        {renderCapabilityChip('filter', resource.query_capabilities.filter)}
                        {renderCapabilityChip('sort', resource.query_capabilities.sort)}
                        {renderCapabilityChip('page', resource.query_capabilities.pagination)}
                        {renderCapabilityChip('embed', resource.query_capabilities.embed)}
                        {renderCapabilityChip('item', resource.query_capabilities.item_route)}
                      </div>
                    </button>
                  ))}
                </div>
              </section>
            ) : null
          )}
          {!groupedResources.table.length &&
            !groupedResources.object.length &&
            !groupedResources.value.length && (
              <p className="overview-empty">No resources match the current search.</p>
            )}
        </div>
      )}
    </aside>
  );
}

function ExplorerHeader({
  resource,
  selectedRow,
  readonly,
  view,
  actions,
  onChangeView,
  onOpenCreate,
  onOpenEdit,
  onOpenDelete
}: {
  resource: ResourceOverview | null;
  selectedRow: Record<string, unknown> | null;
  readonly: boolean;
  view: OverviewUrlState['view'];
  actions: ReturnType<typeof getVisibleMutationActions>;
  onChangeView: (view: OverviewUrlState['view']) => void;
  onOpenCreate: () => void;
  onOpenEdit: () => void;
  onOpenDelete: () => void;
}) {
  const collectionRoute = resource ? `/${resource.name}` : '/';
  const sampleItemRoute =
    resource?.sample_item_id && resource.primary_key ? `/${resource.name}/${resource.sample_item_id}` : null;
  const selectedItemRoute =
    resource?.primary_key && selectedRow?.[resource.primary_key] !== undefined
      ? `/${resource.name}/${String(selectedRow[resource.primary_key])}`
      : null;

  return (
    <div className="explorer-header">
      <div className="explorer-header-summary">
        <div>
          <p className="section-title">Explorer</p>
          <h2>{resource?.name ?? 'Choose a resource'}</h2>
        </div>
        {resource && (
          <div className="resource-summary-row">
            <span className="overview-kind-badge">{resource.kind}</span>
            {resource.primary_key && <span className="overview-inline-badge">Primary key: {resource.primary_key}</span>}
            {resource.row_count !== null ? (
              <span className="overview-inline-badge">{resource.row_count} rows</span>
            ) : resource.key_count !== null ? (
              <span className="overview-inline-badge">{resource.key_count} keys</span>
            ) : (
              <span className="overview-inline-badge">Scalar value</span>
            )}
          </div>
        )}
        {resource && (
          <div className="route-link-row">
            <a href={collectionRoute} target="_blank" rel="noreferrer">
              Collection route
            </a>
            {sampleItemRoute && (
              <a href={sampleItemRoute} target="_blank" rel="noreferrer">
                Sample item
              </a>
            )}
            {selectedItemRoute && (
              <a href={selectedItemRoute} target="_blank" rel="noreferrer">
                Selected item
              </a>
            )}
          </div>
        )}
      </div>

      <div className="explorer-header-actions">
        <div className="view-toggle" role="tablist" aria-label="Explorer view">
          <button type="button" className={view === 'explore' ? 'is-active' : ''} onClick={() => onChangeView('explore')}>
            Explore
          </button>
          <button type="button" className={view === 'raw' ? 'is-active' : ''} onClick={() => onChangeView('raw')}>
            Raw JSON
          </button>
        </div>

        {!readonly && (
          <div className="mutation-action-row">
            {actions.createRow && (
              <button type="button" className="overview-secondary-button" onClick={onOpenCreate}>
                New row
              </button>
            )}
            {(actions.editRow || actions.editObject) && (
              <button type="button" className="overview-secondary-button" onClick={onOpenEdit}>
                {resource?.kind === 'object' ? 'Edit object' : 'Edit row'}
              </button>
            )}
            {actions.deleteRow && (
              <button type="button" className="overview-secondary-button is-danger" onClick={onOpenDelete}>
                Delete row
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function QuerySummaryBar({
  chips,
  hasState,
  onClear,
  onRemoveChip
}: {
  chips: QuerySummaryChip[];
  hasState: boolean;
  onClear: () => void;
  onRemoveChip: (chip: QuerySummaryChip) => void;
}) {
  return (
    <section className="query-summary-bar" data-testid="query-summary">
      <div className="query-summary-head">
        <div>
          <p className="section-title">Query summary</p>
          <p className="overview-copy">
            Filters, sorting, embeds, and pagination always compile back to the server&apos;s native REST query params.
          </p>
        </div>
        <button type="button" className="overview-secondary-button" onClick={onClear} disabled={!hasState}>
          Clear all
        </button>
      </div>
      {chips.length > 0 ? (
        <div className="query-chip-row">
          {chips.map((chip) => (
            <button
              key={chip.id}
              type="button"
              className={`query-chip is-${chip.kind}`}
              onClick={() => onRemoveChip(chip)}
              aria-label={chip.removeLabel}
            >
              <strong>{chip.label}</strong>
              <span>{chip.value}</span>
            </button>
          ))}
        </div>
      ) : (
        <p className="overview-empty">No active filters, sorting, or embeds.</p>
      )}
    </section>
  );
}

function DataExplorerPanel({
  resource,
  response,
  error,
  isLoading,
  state,
  selectedRow,
  rawMode,
  columnVisibility,
  onColumnVisibilityChange,
  onStateChange,
  onRowSelect
}: {
  resource: ResourceOverview | null;
  response: ResourceResponse | undefined;
  error: Error | null;
  isLoading: boolean;
  state: OverviewUrlState;
  selectedRow: Record<string, unknown> | null;
  rawMode: boolean;
  columnVisibility: VisibilityState;
  onColumnVisibilityChange: (resourceName: string, updater: Updater<VisibilityState>) => void;
  onStateChange: (updater: (state: OverviewUrlState) => OverviewUrlState) => void;
  onRowSelect: (row: Record<string, unknown>) => void;
}) {
  const rows = resource?.kind === 'table' ? getTableRows(response?.parsed) : [];
  const columnNames = resource?.kind === 'table' ? getColumnNames(resource, rows) : [];
  const fieldOptions = columnNames.length > 0 ? columnNames : ['id'];
  const relationColumns = resource?.columns.filter((column) => Boolean(column.relation)) ?? [];
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
      )
    };
  });

  const table = useReactTable({
    data: rows,
    columns,
    getCoreRowModel: getCoreRowModel(),
    manualPagination: true,
    manualSorting: true,
    state: {
      columnVisibility,
      sorting: state.sorting.map((sort) => ({ id: sort.id, desc: sort.desc }))
    },
    pageCount: isPaginatedResponse(response?.parsed) ? response.parsed.pages : 1,
    onColumnVisibilityChange: (updater) => {
      if (!resource) {
        return;
      }
      onColumnVisibilityChange(resource.name, updater);
    }
  });

  if (!resource) {
    return <p className="overview-empty">Choose a resource to start exploring.</p>;
  }

  if (error) {
    return (
      <div className="error-state">
        <p className="section-title">Request failed</p>
        <pre className="json-viewer">{error.message}</pre>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="data-explorer-stack">
        <div className="control-bar">
          <div className="skeleton skeleton-bar" />
          <div className="skeleton skeleton-bar short" />
        </div>
        <div className="table-shell">
          <TableSkeleton />
        </div>
      </div>
    );
  }

  if (rawMode) {
    return (
      <div className="data-explorer-stack">
        {resource.kind === 'table' && (
          <ControlBar
            fields={fieldOptions}
            filters={state.filters}
            relationColumns={relationColumns}
            state={state}
            onStateChange={onStateChange}
          />
        )}
        <pre className="json-viewer">{formatJson(response?.parsed ?? null)}</pre>
      </div>
    );
  }

  if (resource.kind !== 'table') {
    return (
      <div className="non-table-panel" data-testid="non-table-view">
        <p className="overview-copy">
          This resource is not array-shaped, so the explorer stays JSON-first.
        </p>
        <pre className="json-viewer">{formatJson(response?.parsed ?? null)}</pre>
      </div>
    );
  }

  return (
    <div className="data-explorer-stack">
      <ControlBar
        fields={fieldOptions}
        filters={state.filters}
        relationColumns={relationColumns}
        state={state}
        onStateChange={onStateChange}
      />

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
            {rows.length === 0 && (
              <tr>
                <td colSpan={columnNames.length || 1}>
                  <p className="overview-empty">No rows match the current query.</p>
                </td>
              </tr>
            )}
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
            ? `Page ${response.parsed.page} of ${response.parsed.pages}`
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

function ControlBar({
  fields,
  filters,
  relationColumns,
  state,
  onStateChange
}: {
  fields: string[];
  filters: FilterDescriptor[];
  relationColumns: ResourceOverview['columns'];
  state: OverviewUrlState;
  onStateChange: (updater: (state: OverviewUrlState) => OverviewUrlState) => void;
}) {
  return (
    <div className="control-bar">
      <FilterBuilder
        fields={fields}
        filters={filters}
        onAddFilter={() =>
          onStateChange((current) => ({
            ...current,
            page: 1,
            filters: [...current.filters, createFilter(fields[0] ?? 'id')]
          }))
        }
        onChangeFilter={(filterId, patch) =>
          onStateChange((current) => ({
            ...current,
            page: 1,
            filters: current.filters.map((filter) => (filter.id === filterId ? { ...filter, ...patch } : filter))
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
          <p className="overview-copy">Choose user-facing operators; the request still uses the server-native query params.</p>
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
                  {FILTER_OPERATOR_LABELS[operator]}
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

      <details className="filter-help">
        <summary>Advanced operator semantics</summary>
        <p className="overview-copy">
          `contains`, `starts with`, `ends with`, null checks, and comparison operators map to the server&apos;s
          native `field:operator=value` query syntax.
        </p>
      </details>
    </div>
  );
}

function InspectorPanel({
  resource,
  response,
  schemaDraft,
  schemaStatus,
  schemaDiffSummary,
  schemaValidationError,
  selectedRow,
  selectedTab,
  outgoingRelations,
  incomingRelations,
  readonly,
  mobileOpen,
  schemaBusy,
  canSaveSchema,
  canInferSchema,
  requestPath,
  requestUrl,
  onTabChange,
  onSchemaDraftChange,
  onCopy,
  onOpenRequest,
  onReloadSchema,
  onSaveSchema,
  onInferSchema,
  onDrilldownOutgoing,
  onDrilldownIncoming
}: {
  resource: ResourceOverview | null;
  response: ResourceResponse | undefined;
  schemaDraft: string;
  schemaStatus: string | null;
  schemaDiffSummary: string[];
  schemaValidationError: string | null;
  selectedRow: Record<string, unknown> | null;
  selectedTab: InspectorTab;
  outgoingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  incomingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  readonly: boolean;
  mobileOpen: boolean;
  schemaBusy: boolean;
  canSaveSchema: boolean;
  canInferSchema: boolean;
  requestPath: string;
  requestUrl: string;
  onTabChange: (tab: InspectorTab) => void;
  onSchemaDraftChange: (value: string) => void;
  onCopy: (value: string) => Promise<void>;
  onOpenRequest: () => void;
  onReloadSchema: () => void;
  onSaveSchema: () => void;
  onInferSchema: () => void;
  onDrilldownOutgoing: (entry: { relation: OverviewRelation; value: string | null }) => void;
  onDrilldownIncoming: (entry: { relation: OverviewRelation; value: string | null }) => void;
}) {
  const curlExample = `curl -H 'Accept: application/json' '${requestUrl}'`;

  return (
    <aside
      className={`workspace-details shell-card ${mobileOpen ? 'mobile-sheet-open' : ''}`}
      data-testid="inspector-panel"
    >
      <div className="overview-panel-head">
        <div>
          <p className="section-title">Inspector</p>
          <h2>Request, selection, schema</h2>
        </div>
      </div>

      <div className="inspector-tab-row" role="tablist" aria-label="Inspector tabs">
        <button type="button" className={selectedTab === 'request' ? 'is-active' : ''} onClick={() => onTabChange('request')}>
          Request
        </button>
        <button
          type="button"
          className={selectedTab === 'selection' ? 'is-active' : ''}
          onClick={() => onTabChange('selection')}
        >
          Selection
        </button>
        <button type="button" className={selectedTab === 'schema' ? 'is-active' : ''} onClick={() => onTabChange('schema')}>
          Schema
        </button>
      </div>

      {selectedTab === 'request' && (
        <section className="inspector-section">
          <div className="request-header-row">
            <span className="overview-inline-code">
              <span className="overview-method">GET</span>
              {requestPath}
            </span>
          </div>
          <div className="request-action-grid">
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(requestPath)}>
              Copy relative URL
            </button>
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(curlExample)}>
              Copy curl
            </button>
            <button type="button" className="overview-secondary-button" onClick={onOpenRequest}>
              Open request
            </button>
          </div>
          <code className="request-path">{requestPath}</code>
          {response && (
            <div className="request-meta-stack">
              <span className={`status-pill ${response.status >= 400 ? 'is-error' : ''}`}>
                {response.status} {response.statusText}
              </span>
              {isPaginatedResponse(response.parsed) && (
                <div className="request-page-metadata">
                  <span className="overview-inline-badge">Page {response.parsed.page}</span>
                  <span className="overview-inline-badge">{response.parsed.items} items</span>
                  <span className="overview-inline-badge">{response.parsed.pages} pages</span>
                </div>
              )}
            </div>
          )}
        </section>
      )}

      {selectedTab === 'selection' && (
        <section className="inspector-section">
          {resource?.kind === 'table' && selectedRow ? (
            <SelectionPanel
              resource={resource}
              row={selectedRow}
              outgoingRelations={outgoingRelations}
              incomingRelations={incomingRelations}
              onDrilldownOutgoing={onDrilldownOutgoing}
              onDrilldownIncoming={onDrilldownIncoming}
            />
          ) : (
            <section className="resource-snapshot">
              <h3>Resource snapshot</h3>
              <p className="overview-copy">
                {resource
                  ? truncate(summarizeValue(response?.parsed ?? resource.row_samples[0] ?? null), 180)
                  : 'Choose a resource to inspect it here.'}
              </p>
              {resource && resource.row_samples.length > 0 && (
                <pre className="json-viewer">{formatJson(resource.row_samples[0])}</pre>
              )}
            </section>
          )}
        </section>
      )}

      {selectedTab === 'schema' && (
        <SchemaEditorPanel
          draft={schemaDraft}
          status={schemaStatus}
          diffSummary={schemaDiffSummary}
          validationError={schemaValidationError}
          readonly={readonly}
          busy={schemaBusy}
          canSave={canSaveSchema}
          canInfer={canInferSchema}
          onChange={onSchemaDraftChange}
          onReload={onReloadSchema}
          onSave={onSaveSchema}
          onInfer={onInferSchema}
        />
      )}
    </aside>
  );
}

function SelectionPanel({
  resource,
  row,
  outgoingRelations,
  incomingRelations,
  onDrilldownOutgoing,
  onDrilldownIncoming
}: {
  resource: ResourceOverview;
  row: Record<string, unknown>;
  outgoingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  incomingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  onDrilldownOutgoing: (entry: { relation: OverviewRelation; value: string | null }) => void;
  onDrilldownIncoming: (entry: { relation: OverviewRelation; value: string | null }) => void;
}) {
  const itemRoute =
    resource.primary_key && row[resource.primary_key] !== undefined
      ? `/${resource.name}/${String(row[resource.primary_key])}`
      : null;

  return (
    <div className="details-stack">
      <section>
        <h3>Selected row</h3>
        <p className="overview-copy">
          {itemRoute ? (
            <>
              Item route: <code className="overview-inline-code">{itemRoute}</code>
            </>
          ) : (
            'Select a row to inspect the raw JSON payload.'
          )}
        </p>
        <pre className="json-viewer">{formatJson(row)}</pre>
      </section>

      <section>
        <h3>Outgoing relations</h3>
        <div className="relation-link-list">
          {outgoingRelations.length > 0 ? (
            outgoingRelations.map((entry) => (
              <button
                key={`${entry.relation.source_table}:${entry.relation.source_column}`}
                type="button"
                className="relation-link-button"
                onClick={() => onDrilldownOutgoing(entry)}
              >
                <strong>{entry.relation.target_table}</strong>
                <span>
                  {entry.relation.target_column} = {entry.value}
                </span>
              </button>
            ))
          ) : (
            <p className="overview-empty">No outgoing relation drill-down is available for this row.</p>
          )}
        </div>
      </section>

      <section>
        <h3>Incoming relations</h3>
        <div className="relation-link-list">
          {incomingRelations.length > 0 ? (
            incomingRelations.map((entry) => (
              <button
                key={`${entry.relation.source_table}:${entry.relation.source_column}:incoming`}
                type="button"
                className="relation-link-button"
                onClick={() => onDrilldownIncoming(entry)}
              >
                <strong>{entry.relation.source_table}</strong>
                <span>
                  {entry.relation.source_column} = {entry.value}
                </span>
              </button>
            ))
          ) : (
            <p className="overview-empty">No incoming relation drill-down is available for this row.</p>
          )}
        </div>
      </section>
    </div>
  );
}

function SchemaEditorPanel({
  draft,
  status,
  diffSummary,
  validationError,
  readonly,
  busy,
  canSave,
  canInfer,
  onChange,
  onReload,
  onSave,
  onInfer
}: {
  draft: string;
  status: string | null;
  diffSummary: string[];
  validationError: string | null;
  readonly: boolean;
  busy: boolean;
  canSave: boolean;
  canInfer: boolean;
  onChange: (value: string) => void;
  onReload: () => void;
  onSave: () => void;
  onInfer: () => void;
}) {
  return (
    <section className="inspector-section">
      <div className="schema-panel-head">
        <div>
          <h3>Schema editor</h3>
          <p className="overview-copy">
            Edit the JSON schema overlay directly. Save uses `PUT /schema`; infer uses `POST /schema/infer`.
          </p>
        </div>
        {readonly && <span className="status-pill is-warn">Read-only mode</span>}
      </div>

      <textarea
        className="schema-editor"
        value={draft}
        onChange={(event) => onChange(event.target.value)}
        readOnly={readonly}
        spellCheck={false}
        data-testid="schema-editor"
      />

      {validationError && <p className="copy-status is-error">{validationError}</p>}
      {diffSummary.length > 0 && (
        <div className="schema-diff-summary">
          <p className="section-title">Diff summary</p>
          {diffSummary.map((line) => (
            <p key={line} className="overview-copy">
              {line}
            </p>
          ))}
        </div>
      )}

      <div className="schema-editor-actions">
        <button type="button" className="overview-secondary-button" onClick={onReload}>
          Reload
        </button>
        <button
          type="button"
          className="overview-secondary-button"
          onClick={onInfer}
          disabled={readonly || !canInfer || busy}
        >
          {busy ? 'Working…' : 'Infer from data'}
        </button>
        <button
          type="button"
          className="overview-secondary-button"
          onClick={onSave}
          disabled={readonly || !canSave || busy}
        >
          {busy ? 'Working…' : 'Save'}
        </button>
      </div>

      {status && <p className="copy-status">{status}</p>}
    </section>
  );
}

function MutationDialog({
  open,
  mode,
  resource,
  selectedRow,
  objectValue,
  onClose,
  onSubmit
}: {
  open: boolean;
  mode: 'create' | 'edit' | 'delete' | 'editObject' | null;
  resource: ResourceOverview | null;
  selectedRow: Record<string, unknown> | null;
  objectValue: unknown;
  onClose: () => void;
  onSubmit: (plan: MutationPlan) => Promise<void>;
}) {
  const [draftText, setDraftText] = useState('{}');
  const [replaceFullItem, setReplaceFullItem] = useState(false);
  const [confirmAction, setConfirmAction] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (!open || !resource || !mode) {
      return;
    }
    const sourceValue =
      mode === 'create'
        ? {}
        : mode === 'editObject'
          ? objectValue ?? {}
          : selectedRow ?? {};
    setDraftText(formatJson(sourceValue));
    setReplaceFullItem(false);
    setConfirmAction(false);
    setError(null);
  }, [mode, objectValue, open, resource, selectedRow]);

  if (!open || !resource || !mode) {
    return null;
  }

  const sourceValue =
    mode === 'create' ? {} : mode === 'editObject' ? objectValue ?? {} : selectedRow ?? {};
  const validationError = mode === 'delete' ? null : getJsonValidationError(draftText);

  let plan: MutationPlan | null = null;
  let planError: string | null = null;
  try {
    plan = buildMutationPlan({
      resource,
      mode,
      originalValue: sourceValue,
      draftText,
      replaceFullItem
    });
  } catch (caught) {
    planError = caught instanceof Error ? caught.message : 'Unable to build mutation request.';
  }

  const submitDisabled =
    pending ||
    Boolean(validationError) ||
    Boolean(planError) ||
    !plan ||
    (plan?.method === 'PATCH' && plan.changedKeys.length === 0) ||
    ((plan?.requiresConfirmation ?? false) && !confirmAction);

  return (
    <div className="dialog-backdrop" role="presentation" onClick={onClose}>
      <div
        className="mutation-dialog"
        role="dialog"
        aria-modal="true"
        data-testid="mutation-dialog"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="overview-panel-head">
          <div>
            <p className="section-title">Mutation</p>
            <h2>{renderMutationTitle(mode, resource.kind)}</h2>
          </div>
          <button type="button" className="overview-icon-button" onClick={onClose}>
            Close
          </button>
        </div>

        {mode !== 'delete' && (
          <>
            <textarea
              className="schema-editor mutation-editor"
              value={draftText}
              onChange={(event) => setDraftText(event.target.value)}
              spellCheck={false}
            />
            {(mode === 'edit' || mode === 'editObject') && (
              <label className="mutation-toggle">
                <input
                  type="checkbox"
                  checked={replaceFullItem}
                  onChange={(event) => {
                    setReplaceFullItem(event.target.checked);
                    setConfirmAction(false);
                  }}
                />
                <span>Replace the full document with `PUT` instead of sending only changed keys with `PATCH`.</span>
              </label>
            )}
          </>
        )}

        {plan && (
          <div className="mutation-plan-card">
            <p className="section-title">Request</p>
            <code className="request-path">
              {plan.method} {plan.path}
            </code>
            {plan.changedKeys.length > 0 && <p className="overview-copy">Changed keys: {plan.changedKeys.join(', ')}</p>}
            {plan.method === 'PATCH' && plan.changedKeys.length === 0 && (
              <p className="copy-status is-error">No changed keys detected. Edit the JSON or switch to full replace.</p>
            )}
          </div>
        )}

        {(plan?.requiresConfirmation ?? false) && (
          <label className="mutation-toggle">
            <input
              type="checkbox"
              checked={confirmAction}
              onChange={(event) => setConfirmAction(event.target.checked)}
            />
            <span>
              {plan?.method === 'DELETE'
                ? 'I understand this delete cannot be undone.'
                : 'I understand this will replace the full document with PUT.'}
            </span>
          </label>
        )}

        {(validationError || planError || error) && (
          <p className="copy-status is-error">{validationError ?? planError ?? error}</p>
        )}

        <div className="mutation-dialog-actions">
          <button type="button" className="overview-secondary-button" onClick={onClose} disabled={pending}>
            Cancel
          </button>
          <button
            type="button"
            className="overview-secondary-button"
            onClick={async () => {
              if (!plan || submitDisabled) {
                return;
              }
              setPending(true);
              setError(null);
              try {
                await onSubmit(plan);
                onClose();
              } catch (caught) {
                setError(caught instanceof Error ? caught.message : 'Request failed.');
              } finally {
                setPending(false);
              }
            }}
            disabled={submitDisabled}
          >
            {pending ? 'Submitting…' : `${plan?.method ?? 'Submit'} request`}
          </button>
        </div>
      </div>
    </div>
  );
}

function RelationMap({
  overview,
  selectedResourceName,
  onSelectResource,
  loading
}: {
  overview: OverviewPageData | null;
  selectedResourceName: string | null;
  onSelectResource: (resourceName: string) => void;
  loading: boolean;
}) {
  if (loading) {
    return <div className="skeleton relation-map-skeleton" />;
  }
  if (!overview || overview.resources.length === 0) {
    return <p className="overview-empty">No resources are available yet.</p>;
  }

  const columns = Math.max(1, Math.ceil(Math.sqrt(overview.resources.length)));
  const nodes: Node[] = overview.resources.map((resource, index) => ({
    id: resource.name,
    position: {
      x: (index % columns) * 280,
      y: Math.floor(index / columns) * 180
    },
    draggable: false,
    data: {
      label: (
        <div className={`graph-node-card ${resource.name === selectedResourceName ? 'is-selected' : ''}`}>
          <div className="graph-node-head">
            <strong>{resource.name}</strong>
            <span className="overview-kind-badge">{resource.kind}</span>
          </div>
          <p>
            {resource.row_count !== null
              ? `${resource.row_count} rows`
              : resource.key_count !== null
                ? `${resource.key_count} keys`
                : 'scalar value'}
          </p>
        </div>
      )
    },
    selectable: false
  }));

  const edges: Edge[] = overview.edges.map((edge) => ({
    id: `${edge.source_table}:${edge.source_column}:${edge.target_table}:${edge.target_column}`,
    source: edge.source_table,
    target: edge.target_table,
    label: `${edge.source_column} -> ${edge.target_column}`,
    animated: edge.source_table === selectedResourceName || edge.target_table === selectedResourceName,
    style:
      edge.source_table === selectedResourceName || edge.target_table === selectedResourceName
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
        nodesConnectable={false}
        elementsSelectable={false}
        onNodeClick={(_, node) => onSelectResource(node.id)}
      >
        <MiniMap zoomable pannable className="relation-map-minimap" />
        <Controls showInteractive={false} />
        <Background gap={20} size={1} color="rgba(57, 85, 77, 0.12)" />
      </ReactFlow>
    </div>
  );
}

function ToastViewport({ toasts }: { toasts: ToastMessage[] }) {
  return (
    <div className="toast-viewport" aria-live="polite" aria-atomic="true">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast-message is-${toast.tone}`}>
          {toast.message}
        </div>
      ))}
    </div>
  );
}

function HighlightText({ text, needle }: { text: string; needle: string }) {
  if (!needle.trim()) {
    return <>{text}</>;
  }

  const lowerText = text.toLowerCase();
  const lowerNeedle = needle.trim().toLowerCase();
  const index = lowerText.indexOf(lowerNeedle);
  if (index === -1) {
    return <>{text}</>;
  }

  const start = text.slice(0, index);
  const match = text.slice(index, index + lowerNeedle.length);
  const end = text.slice(index + lowerNeedle.length);
  return (
    <>
      {start}
      <mark>{match}</mark>
      {end}
    </>
  );
}

function TableSkeleton() {
  return (
    <div className="table-skeleton">
      {Array.from({ length: 6 }).map((_, rowIndex) => (
        <div key={rowIndex} className="table-skeleton-row">
          {Array.from({ length: 5 }).map((__, cellIndex) => (
            <div key={cellIndex} className="skeleton skeleton-cell" />
          ))}
        </div>
      ))}
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

function groupResources(resources: ResourceOverview[]) {
  return resources.reduce<Record<ResourceOverview['kind'], ResourceOverview[]>>(
    (groups, resource) => {
      groups[resource.kind].push(resource);
      return groups;
    },
    { table: [], object: [], value: [] }
  );
}

function renderCapabilityChip(label: string, enabled: boolean) {
  return (
    <span className={`capability-chip ${enabled ? 'is-enabled' : ''}`} key={label}>
      {label}
    </span>
  );
}

function renderLiveUpdateLabel(status: LiveUpdateStatus) {
  switch (status) {
    case 'live':
      return 'Live';
    case 'reconnecting':
      return 'Reconnecting';
    case 'paused':
      return 'Paused';
    default:
      return 'Connecting';
  }
}

function getJsonValidationError(value: string) {
  try {
    JSON.parse(value);
    return null;
  } catch (caught) {
    return caught instanceof Error ? caught.message : 'Invalid JSON.';
  }
}

function buildDrilldownFilter(field: string, value: string | null): FilterDescriptor {
  return {
    id: `${field}:eq:drilldown`,
    field,
    operator: 'eq',
    value: value ?? ''
  };
}

function renderMutationTitle(mode: NonNullable<OverviewUiState['mutationDialog']['mode']>, kind: ResourceOverview['kind']) {
  if (mode === 'create') {
    return 'Create row';
  }
  if (mode === 'delete') {
    return 'Delete row';
  }
  if (kind === 'object') {
    return 'Edit object';
  }
  return 'Edit row';
}
