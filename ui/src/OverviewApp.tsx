import { QueryClient, QueryClientProvider, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { type Updater, type VisibilityState } from '@tanstack/react-table';
import { ReactFlowProvider, type Connection } from '@xyflow/react';
import { startTransition, useDeferredValue, useEffect, useState } from 'react';

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
  getTableRows,
  isRecord
} from './helpers';
import {
  buildQuerySummaryChips,
  getVisibleMutationActions,
  loadOverviewPreferences,
  saveOverviewPreferences,
  summarizeSchemaDiff
} from './overviewUtils';
import { DataExplorerPanel, ExplorerHeader, QuerySummaryBar, ResourceSidebar } from './overview/explorer';
import { InspectorPanel, MutationDialog } from './overview/inspector';
import { RelationMap } from './overview/relationMap';
import {
  SummaryCard,
  ToastViewport,
  type ToastMessage,
  getJsonValidationError,
  groupResources,
  renderLiveUpdateLabel
} from './overview/shared';
import {
  deriveSchemaEdges,
  parseSchemaConnection,
  parseSchemaDocument,
  upsertSchemaRelationship
} from './schemaEditor';
import type {
  FilterDescriptor,
  InspectorTab,
  MutationPlan,
  OverviewPreferences,
  OverviewUiState,
  OverviewUrlState
} from './types';
import {
  buildBrowserQueryString,
  buildResourceRequestPath,
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
  const [schemaDraftDirty, setSchemaDraftDirty] = useState(false);
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
      setSchemaDraftDirty(false);
      setSchemaStatus('Schema saved.');
      pushToast('Saved schema changes.', 'success');
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
      setSchemaDraftDirty(false);
      setSchemaStatus(`Schema inferred${result.path ? ` to ${result.path}` : ''}.`);
      pushToast(`Schema inference completed${result.path ? `: ${result.path}` : '.'}`, 'success');
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
  const parsedSchemaDraft = parseSchemaDocument(schemaDraft);
  const hasUsableSchemaDraft = schemaDraftDirty || Boolean(schemaQuery.data);
  const schemaEdges =
    hasUsableSchemaDraft && parsedSchemaDraft.document
      ? deriveSchemaEdges(parsedSchemaDraft.document, resources)
      : overviewQuery.data?.edges ?? [];
  const schemaValidationError = parsedSchemaDraft.error ?? getJsonValidationError(schemaDraft);

  useEffect(() => {
    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, []);

  useEffect(() => {
    saveOverviewPreferences(window.localStorage, preferences);
  }, [preferences]);

  useEffect(() => {
    if (!schemaQuery.data || schemaDraftDirty) {
      return;
    }
    const nextText = formatJson(schemaQuery.data);
    setLoadedSchemaText(nextText);
    setSchemaDraft(nextText);
  }, [schemaDraftDirty, schemaQuery.data]);

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
    let refreshTimer: number | null = null;
    let stormPauseNotified = false;
    let eventTimestamps: number[] = [];

    function flushRefresh() {
      refreshTimer = null;
      void client.invalidateQueries({ queryKey: ['overview'] });
      void client.invalidateQueries({ queryKey: ['resource'] });
      void client.invalidateQueries({ queryKey: ['schema'] });
    }

    function pauseLiveUpdates(message: string) {
      source?.close();
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
        refreshTimer = null;
      }
      setUiState((current) => ({ ...current, liveUpdates: 'paused' }));
      if (!stormPauseNotified) {
        pushToast(message, 'error');
        stormPauseNotified = true;
      }
    }

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
        stormPauseNotified = false;
        eventTimestamps = [];
        setUiState((current) => ({ ...current, liveUpdates: 'live' }));
        if (wasReconnecting) {
          pushToast('Reconnected to live updates.', 'success');
        }
      };

      const handleServerEvent = (label: string) => {
        const now = Date.now();
        eventTimestamps = eventTimestamps.filter((timestamp) => now - timestamp < 2_000);
        eventTimestamps.push(now);

        if (eventTimestamps.length >= 12) {
          pauseLiveUpdates('Live updates paused due to an event storm.');
          return;
        }

        if (refreshTimer !== null) {
          return;
        }
        refreshTimer = window.setTimeout(flushRefresh, 250);
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
          pauseLiveUpdates('Live updates paused.');
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
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
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
            Click a node to switch resources. Drag from a source column to a target column to stage
            a schema relationship before saving it.
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
          onSchemaDraftChange={updateSchemaDraft}
          onCopy={copyText}
          onOpenRequest={() => window.open(requestUrl, '_blank', 'noopener,noreferrer')}
          onReloadSchema={reloadSchemaDraft}
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

  function updateSchemaDraft(nextDraft: string) {
    setSchemaDraft(nextDraft);
    setSchemaDraftDirty(true);
    setSchemaStatus(null);
  }

  function reloadSchemaDraft() {
    if (schemaQuery.data) {
      const nextText = formatJson(schemaQuery.data);
      setLoadedSchemaText(nextText);
      setSchemaDraft(nextText);
    }
    setSchemaDraftDirty(false);
    setSchemaStatus('Schema reloaded from the server.');
    void client.invalidateQueries({ queryKey: ['schema'] });
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
    updateSchemaDraft(formatJson(nextSchema));
    setSchemaStatus(
      `Staged ${parsedConnection.sourceTable}.${parsedConnection.sourceColumn} -> ${parsedConnection.targetTable}.${parsedConnection.targetColumn}. Save schema to persist it.`
    );
    setInspectorTab('schema');
    setPreferences((current) => ({ ...current, mobileSurface: 'inspector' }));
  }

  function pushToast(message: string, tone: ToastMessage['tone']) {
    const id = Date.now() + Math.floor(Math.random() * 1000);
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 3_000);
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
