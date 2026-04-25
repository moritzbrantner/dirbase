import { QueryClient, QueryClientProvider, useQuery, useQueryClient } from '@tanstack/react-query';
import { type Updater, type VisibilityState } from '@tanstack/react-table';
import { ReactFlowProvider } from '@xyflow/react';
import { useDeferredValue, useEffect, useState } from 'react';

import { fetchOverview, fetchResource, fetchSchemaEditor, mutateResource } from './api';
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
import { SchemaWorkspace } from './overview/schema/SchemaWorkspace';
import { ToastViewport, groupResources, renderLiveUpdateLabel } from './overview/shared';
import { useOverviewLiveUpdates } from './overview/useOverviewLiveUpdates';
import { useOverviewSchemaWorkspace } from './overview/useOverviewSchemaWorkspace';
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
  const schemaEditorQuery = useQuery({
    queryKey: ['schema-editor'],
    queryFn: () => fetchSchemaEditor()
  });

  const resources = overviewQuery.data?.resources ?? [];
  const selectedResource = findSelectedResource(resources, urlState.resource);
  const serverCapabilities = overviewQuery.data?.server_capabilities ?? null;
  const requestPath = buildResourceRequestPath(selectedResource, urlState);
  const requestUrl = `${window.location.origin}${requestPath}`;

  const resourceQuery = useQuery({
    queryKey: ['resource', requestPath],
    queryFn: () => fetchResource(requestPath),
    enabled: Boolean(selectedResource && urlState.mode === 'data')
  });

  const { liveUpdates, retryLiveUpdates } = useOverviewLiveUpdates({
    client,
    onToast: pushToast
  });
  const {
    declaredDraft,
    declaredDraftText,
    effectiveSchema,
    inferredSchema,
    schemaStatus,
    jsonDraftError,
    schemaValidationError,
    schemaBusy,
    schemaDirty,
    schemaStale,
    jsonDrawerOpen,
    setJsonDrawerOpen,
    applyDeclaredUpdate,
    updateDeclaredDraftText,
    reloadSchemaDraft,
    discardInvalidJsonChanges,
    saveSchema,
    inferSchema,
    savePath
  } = useOverviewSchemaWorkspace({
    client,
    schemaEditorData: schemaEditorQuery.data,
    onToast: pushToast
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
  const hasActiveQuery = urlState.filters.length > 0 || urlState.sorting.length > 0 || urlState.embeds.length > 0;

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
    commitUrlState(resetTableState(selectedResource.name, urlState.view, urlState.mode));
  }, [commitUrlState, selectedResource, urlState.mode, urlState.resource, urlState.view]);

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
      commitUrlState(resetTableState(selectedResource.name, urlState.view, urlState.mode));
    }
  }, [
    commitUrlState,
    selectedResource,
    urlState.embeds.length,
    urlState.filters.length,
    urlState.page,
    urlState.perPage,
    urlState.sorting.length,
    urlState.mode,
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
      <section className="overview-status-card shell-card">
        <div className="grid min-w-0 gap-2">
          <div>
            <p className="section-title">Workspace</p>
            <h1 className="text-2xl font-semibold tracking-tight text-stoneink-900">
              {selectedResource?.name ?? 'dirbase overview'}
            </h1>
          </div>
          <p className="overview-copy">
            {overviewQuery.data?.source_rule ?? 'Loading source metadata...'}
          </p>
          <code className="overview-source-line">{overviewQuery.data?.source_label ?? 'Loading source...'}</code>
        </div>
        <div className="overview-status-group">
          <div className="view-toggle" role="tablist" aria-label="Workspace mode">
            <button
              type="button"
              className={urlState.mode === 'data' ? 'is-active' : ''}
              onClick={() => setOverviewMode('data')}
            >
              Data
            </button>
            <button
              type="button"
              className={urlState.mode === 'schema' ? 'is-active' : ''}
              onClick={() => setOverviewMode('schema')}
            >
              Schema
            </button>
          </div>
          <span className="overview-inline-badge">
            {overviewQuery.data?.stats.resource_count ?? 0} resources
          </span>
          <span className="overview-inline-badge">
            {overviewQuery.data?.stats.relation_count ?? 0} relations
          </span>
          <span className="overview-inline-badge">{overviewQuery.data?.stats.total_rows ?? 0} rows</span>
          <span className={`status-pill is-live-${liveUpdates}`}>
            Live {renderLiveUpdateLabel(liveUpdates)}
          </span>
          {uiState.readonly && <span className="status-pill is-warn">Read-only mode</span>}
          {overviewQuery.data?.schema_enabled && <span className="status-pill">Schema loaded</span>}
          {liveUpdates === 'paused' && (
            <button type="button" className="overview-secondary-button" onClick={retryLiveUpdates}>
              Retry live updates
            </button>
          )}
          {urlState.mode === 'data' && (
            <button type="button" className="overview-secondary-button" onClick={() => toggleMobileSurface('map')}>
              {preferences.mobileSurface === 'map' ? 'Close map' : 'Open map'}
            </button>
          )}
        </div>
      </section>

      {urlState.mode === 'data' ? (
        <>
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
                hasState={hasActiveQuery}
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
              selectedRow={uiState.selectedRow}
              selectedTab={uiState.inspectorTab}
              outgoingRelations={outgoingRelationLinks}
              incomingRelations={incomingRelationLinks}
              readonly={uiState.readonly}
              mobileOpen={preferences.mobileSurface === 'inspector'}
              requestPath={requestPath}
              requestUrl={requestUrl}
              onTabChange={setInspectorTab}
              onCopy={copyText}
              onOpenRequest={() => window.open(requestUrl, '_blank', 'noopener,noreferrer')}
              onDrilldownOutgoing={({ relation, value }) =>
                selectResource(relation.target_table, [buildDrilldownFilter(relation.target_column, value)])
              }
              onDrilldownIncoming={({ relation, value }) =>
                selectResource(relation.source_table, [buildDrilldownFilter(relation.source_column, value)])
              }
            />
          </section>

          <section
            className={`relation-map-panel shell-card ${preferences.mobileSurface === 'map' ? 'mobile-drawer-open' : ''}`}
          >
            <div className="grid gap-3">
              <div className="overview-panel-head">
                <div>
                  <p className="section-title">Relation map</p>
                  <h2 className="text-xl font-semibold tracking-tight text-stoneink-900">Links across resources</h2>
                </div>
                <button type="button" className="overview-icon-button" onClick={() => toggleMobileSurface('map')}>
                  Close
                </button>
              </div>
              <p className="overview-copy">
                Click a node to switch resources. Switch to Schema mode to stage and save relationship edits.
              </p>
              <RelationMap
                overview={overviewQuery.data ?? null}
                schemaEdges={overviewQuery.data?.edges ?? []}
                selectedResourceName={selectedResource?.name ?? null}
                onSelectResource={selectResource}
                onCreateRelationship={() => {}}
                connectable={false}
                loading={overviewQuery.isLoading}
              />
            </div>
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
        </>
      ) : (
        <SchemaWorkspace
          resources={resources}
          inferredSchema={inferredSchema}
          effectiveSchema={effectiveSchema}
          declaredDraft={declaredDraft}
          declaredDraftText={declaredDraftText}
          schemaDirty={schemaDirty}
          schemaStale={schemaStale}
          schemaBusy={schemaBusy}
          schemaStatus={schemaStatus}
          jsonDraftError={jsonDraftError}
          schemaValidationError={schemaValidationError}
          jsonDrawerOpen={jsonDrawerOpen}
          savePath={savePath}
          readonly={uiState.readonly}
          selectedResourceName={selectedResource?.name ?? null}
          schemaMobileSurface={preferences.schemaMobileSurface}
          onSelectResource={(resourceName) => selectResource(resourceName)}
          onSchemaMobileSurfaceChange={(surface) =>
            setPreferences((current) => ({ ...current, schemaMobileSurface: surface }))
          }
          onToggleJsonDrawer={() => {
            if (preferences.schemaMobileSurface === 'json') {
              setPreferences((current) => ({ ...current, schemaMobileSurface: 'graph' }));
              return;
            }
            setJsonDrawerOpen((current) => !current);
          }}
          onDeclaredDraftChange={applyDeclaredUpdate}
          onDeclaredDraftTextChange={updateDeclaredDraftText}
          onReload={reloadSchemaDraft}
          onInfer={inferSchema}
          onSave={saveSchema}
          onDiscardInvalidJson={discardInvalidJsonChanges}
        />
      )}

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
      ...resetTableState(resourceName, urlState.view, urlState.mode),
      filters
    });
    setUiState((current) => ({
      ...current,
      selectedResource: resourceName,
      selectedRow: null,
      inspectorTab: 'request'
    }));
    if (urlState.mode === 'data') {
      setPreferences((current) => ({ ...current, mobileSurface: 'explorer' }));
    }
  }

  function setOverviewMode(mode: 'data' | 'schema') {
    commitUrlState({ ...urlState, mode });
    if (mode === 'data') {
      setPreferences((current) => ({ ...current, mobileSurface: 'explorer' }));
      return;
    }
    setPreferences((current) => ({ ...current, schemaMobileSurface: 'graph' }));
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
        selectedRow: result.parsed as Record<string, unknown>,
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
