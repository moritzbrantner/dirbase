import { QueryClient, QueryClientProvider, useQuery, useQueryClient } from '@tanstack/react-query';
import { type Updater, type VisibilityState } from '@tanstack/react-table';
import { ReactFlowProvider } from '@xyflow/react';
import { useDeferredValue, useEffect, useState } from 'react';

import { fetchOverview, fetchResource, fetchSchema, mutateResource } from './api';
import { coerceIncomingRelationValue, coerceRelationValue, getTableRows, isRecord } from './helpers';
import {
  buildQuerySummaryChips,
  createUiState,
  getVisibleMutationActions,
  loadOverviewPreferences,
  saveOverviewPreferences
} from './overviewUtils';
import { DataExplorerPanel, ExplorerHeader, QuerySummaryBar, ResourceSidebar } from './overview/explorer';
import { InspectorPanel, MutationDialog } from './overview/inspector';
import {
  buildDrilldownFilter,
  filterResourcesBySearch,
  findSelectedResource,
  removeQuerySummaryChip
} from './overview/overviewAppUtils';
import { invalidateOverviewQueries } from './overview/queryClient';
import { RelationMap } from './overview/relationMap';
import {
  SummaryCard,
  ToastViewport,
  groupResources,
  renderLiveUpdateLabel
} from './overview/shared';
import { useOverviewLiveUpdates } from './overview/useOverviewLiveUpdates';
import { useOverviewSchemaEditor } from './overview/useOverviewSchemaEditor';
import { useOverviewToasts } from './overview/useOverviewToasts';
import { useOverviewUrlState } from './overview/useOverviewUrlState';
import type {
  FilterDescriptor,
  InspectorTab,
  MutationPlan,
  OverviewPreferences,
  OverviewUiState,
  OverviewUrlState
} from './types';
import { buildResourceRequestPath, resetTableState } from './urlState';

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
  const { urlState, commitUrlState } = useOverviewUrlState();
  const [sidebarSearch, setSidebarSearch] = useState('');
  const deferredSidebarSearch = useDeferredValue(sidebarSearch);
  const [preferences, setPreferences] = useState<OverviewPreferences>(() =>
    loadOverviewPreferences(window.localStorage)
  );
  const [uiState, setUiState] = useState<OverviewUiState>(() => ({
    ...createUiState(urlState.resource, false),
    inspectorTab: preferences.lastInspectorTab
  }));
  const { toasts, pushToast } = useOverviewToasts();

  const overviewQuery = useQuery({
    queryKey: ['overview', overviewEndpoint],
    queryFn: () => fetchOverview(overviewEndpoint)
  });
  const schemaQuery = useQuery({
    queryKey: ['schema'],
    queryFn: () => fetchSchema()
  });

  const resources = overviewQuery.data?.resources ?? [];
  const selectedResource = findSelectedResource(resources, urlState.resource);
  const serverCapabilities = overviewQuery.data?.server_capabilities ?? null;
  const requestPath = buildResourceRequestPath(selectedResource, urlState);
  const requestUrl = `${window.location.origin}${requestPath}`;

  const resourceQuery = useQuery({
    queryKey: ['resource', requestPath],
    queryFn: () => fetchResource(requestPath),
    enabled: Boolean(selectedResource)
  });

  const { liveUpdates, retryLiveUpdates } = useOverviewLiveUpdates({
    client,
    onToast: pushToast
  });
  const {
    schemaDraft,
    schemaStatus,
    schemaDiffSummary,
    schemaValidationError,
    schemaEdges,
    schemaBusy,
    updateSchemaDraft,
    reloadSchemaDraft,
    saveSchema,
    inferSchema,
    stageRelationship
  } = useOverviewSchemaEditor({
    client,
    resources,
    overviewEdges: overviewQuery.data?.edges ?? [],
    schemaData: schemaQuery.data,
    onToast: pushToast,
    onOpenSchemaInspector() {
      setInspectorTab('schema');
      setPreferences((current) => ({ ...current, mobileSurface: 'inspector' }));
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
  const groupedResources = groupResources(filterResourcesBySearch(resources, deferredSidebarSearch));

  useEffect(() => {
    saveOverviewPreferences(window.localStorage, preferences);
  }, [preferences]);

  useEffect(() => {
    if (!selectedResource) {
      return;
    }
    if (urlState.resource === selectedResource.name) {
      return;
    }
    commitUrlState(resetTableState(selectedResource.name, urlState.view));
  }, [commitUrlState, selectedResource, urlState.resource, urlState.view]);

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
    commitUrlState,
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
          <span className={`status-pill is-live-${liveUpdates}`}>
            Live updates: {renderLiveUpdateLabel(liveUpdates)}
          </span>
          {uiState.readonly && <span className="status-pill is-warn">Read-only mode</span>}
          {overviewQuery.data?.schema_enabled && <span className="status-pill">Schema loaded</span>}
        </div>
        <div className="overview-status-group">
          <code className="overview-source-line">{overviewQuery.data?.source_label ?? 'Loading source...'}</code>
          {liveUpdates === 'paused' && (
            <button type="button" className="overview-secondary-button" onClick={retryLiveUpdates}>
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
          onCreateRelationship={stageRelationship}
          connectable={!schemaValidationError}
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
            onRemoveChip={(chip) => updateTableState((current) => removeQuerySummaryChip(current, chip))}
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
          schemaBusy={schemaBusy}
          canSaveSchema={Boolean(serverCapabilities?.schema_write)}
          canInferSchema={Boolean(serverCapabilities?.schema_infer)}
          requestPath={requestPath}
          requestUrl={requestUrl}
          onTabChange={setInspectorTab}
          onSchemaDraftChange={updateSchemaDraft}
          onCopy={copyText}
          onOpenRequest={() => window.open(requestUrl, '_blank', 'noopener,noreferrer')}
          onReloadSchema={reloadSchemaDraft}
          onSaveSchema={saveSchema}
          onInferSchema={inferSchema}
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

  async function submitMutationPlan(plan: MutationPlan) {
    const result = await mutateResource({
      method: plan.method,
      path: plan.path,
      body: plan.body
    });

    await invalidateOverviewQueries(client);

    if (plan.method === 'DELETE') {
      setUiState((current) => ({
        ...current,
        selectedRow: null,
        inspectorTab: current.inspectorTab === 'selection' ? 'request' : current.inspectorTab
      }));
    } else if (isRecord(result.parsed) && selectedResource?.kind === 'table') {
      setUiState((current) => ({
        ...current,
        selectedRow: result.parsed,
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
}
