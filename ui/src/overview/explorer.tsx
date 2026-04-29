import type { Updater, VisibilityState } from '@tanstack/react-table';
import type { KeyboardEvent, MouseEvent } from 'react';

import { formatJson, getColumnNames, getTableRows, isPaginatedResponse } from '../helpers';
import { FILTER_OPERATOR_LABELS, getVisibleMutationActions } from '../overviewUtils';
import type {
  FilterDescriptor,
  FilterOperator,
  OverviewUrlState,
  QuerySummaryChip,
  ResourceOverview,
  ResourceResponse
} from '../types';
import { FILTER_OPERATORS, PAGE_SIZE_OPTIONS, createFilter, nextSorting } from '../urlState';
import { HighlightText, TableSkeleton, renderCellValue } from './shared';

export function ResourceSidebar({
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
          <h2 className="text-xl font-semibold tracking-tight text-stoneink-900">Browse</h2>
        </div>
        <span className="overview-inline-badge">
          {groupedResources.table.length + groupedResources.object.length + groupedResources.value.length} total
        </span>
      </div>

      <label className="sidebar-search-label" htmlFor="resource-search">
        Search resources
      </label>
      <input
        id="resource-search"
        className="overview-input"
        value={search}
        onChange={(event) => onSearchChange(event.target.value)}
        placeholder="name or field"
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
                          <strong className="text-sm font-semibold text-stoneink-900">
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
                          {resource.primary_key && <span>PK {resource.primary_key}</span>}
                          {resource.outgoing_relations.length > 0 && (
                            <span>{resource.outgoing_relations.length} links</span>
                          )}
                        </div>
                        {resource.field_names.length > 0 && (
                          <div className="resource-field-list">
                            {resource.field_names.slice(0, 3).map((field) => (
                              <span key={field} className="resource-field-pill">
                                <HighlightText text={field} needle={searchNeedle} />
                              </span>
                            ))}
                          </div>
                        )}
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

export function ExplorerHeader({
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
  const createFormRoute = resource?.kind === 'table' ? `/${resource.name}/create` : null;
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
          <h2 className="text-2xl font-semibold tracking-tight text-stoneink-900">
            {resource?.name ?? 'Choose a resource'}
          </h2>
        </div>
        {resource && (
          <>
            <div className="resource-summary-row">
              <span className="overview-kind-badge">{resource.kind}</span>
              {resource.primary_key && <span className="overview-inline-badge">PK {resource.primary_key}</span>}
              <span className="overview-inline-badge">
                {resource.row_count !== null
                  ? `${resource.row_count} rows`
                  : resource.key_count !== null
                    ? `${resource.key_count} keys`
                    : 'Scalar value'}
              </span>
            </div>
            <div className="route-link-row text-sm text-stoneink-700">
              <a className="underline decoration-stone-900/20 underline-offset-4" href={collectionRoute} target="_blank" rel="noreferrer">
                Collection
              </a>
              {createFormRoute && (
                <a className="underline decoration-stone-900/20 underline-offset-4" href={createFormRoute} target="_blank" rel="noreferrer">
                  Create form
                </a>
              )}
              {sampleItemRoute && (
                <a className="underline decoration-stone-900/20 underline-offset-4" href={sampleItemRoute} target="_blank" rel="noreferrer">
                  Sample item
                </a>
              )}
              {selectedItemRoute && (
                <a className="underline decoration-stone-900/20 underline-offset-4" href={selectedItemRoute} target="_blank" rel="noreferrer">
                  Selected item
                </a>
              )}
            </div>
          </>
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

export function QuerySummaryBar({
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
  if (chips.length === 0) {
    return null;
  }

  return (
    <section className="query-summary-bar" data-testid="query-summary">
      <div className="query-summary-head">
        <div>
          <p className="section-title">Active query</p>
          <p className="overview-copy">Every chip maps directly to the request URL.</p>
        </div>
        <button type="button" className="overview-secondary-button" onClick={onClear} disabled={!hasState}>
          Clear all
        </button>
      </div>
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
    </section>
  );
}

export function DataExplorerPanel({
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
  const visibleColumns = columnNames.filter((columnName) => columnVisibility[columnName] !== false);
  const columns = columnNames.map((columnName) => ({
    id: columnName,
    visible: columnVisibility[columnName] !== false,
    onToggle: (_event: unknown) => {
      if (!resource) {
        return;
      }
      onColumnVisibilityChange(resource.name, (current) => ({
        ...current,
        [columnName]: !(current[columnName] ?? true)
      }));
    }
  }));

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
        <div className="result-shell">
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
            columns={columns}
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
        <p className="overview-copy">This resource is JSON-first, so the raw document is the primary view.</p>
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
        columns={columns}
        onStateChange={onStateChange}
      />

      <div className="result-shell">
        <div className="result-shell-head">
          <div>
            <p className="section-title">Rows</p>
            <p className="overview-copy">
              Sort from the header. Shift-click any additional column to keep building a multi-column order.
            </p>
          </div>
          <span className="overview-inline-badge">
            {isPaginatedResponse(response?.parsed) ? `${response.parsed.items} items` : `${rows.length} items`}
          </span>
        </div>

        {visibleColumns.length > 0 ? (
          <div className="result-header-grid" role="group" aria-label="Sort rows">
            {visibleColumns.map((columnName) => {
              const sortIndex = state.sorting.findIndex((entry) => entry.id === columnName);
              const sortEntry = sortIndex >= 0 ? state.sorting[sortIndex] : null;
              return (
                <button
                  key={columnName}
                  type="button"
                  className={`result-header-button ${sortEntry ? 'is-active' : ''}`}
                  aria-pressed={Boolean(sortEntry)}
                  onClick={(event: MouseEvent<HTMLButtonElement>) =>
                    onStateChange((current) => ({
                      ...current,
                      page: 1,
                      sorting: nextSorting(current.sorting, columnName, event.shiftKey)
                    }))
                  }
                >
                  <span className="result-header-label">{columnName}</span>
                  <span className="sort-indicator">{formatSortIndicator(sortEntry, sortIndex)}</span>
                </button>
              );
            })}
          </div>
        ) : (
          <p className="overview-empty">No visible columns. Use the column picker to show fields again.</p>
        )}

        {rows.length > 0 ? (
          <div className="result-card-list">
            {rows.map((row, index) => {
              const isSelected =
                primaryKey && selectedPrimaryValue !== undefined
                  ? row[primaryKey] === selectedPrimaryValue
                  : selectedRow === row;
              return (
                <div
                  key={buildRowKey(row, index, primaryKey)}
                  className={`result-card ${isSelected ? 'is-selected' : ''}`}
                  role="button"
                  tabIndex={0}
                  onClick={() => onRowSelect(row)}
                  onKeyDown={(event) => handleRowKeyDown(event, row, onRowSelect)}
                >
                  <div className="result-card-head">
                    <div className="result-card-title">
                      <span className="section-title">Row</span>
                      <strong className="text-sm font-semibold text-stoneink-900">
                        {primaryKey ? `${primaryKey}: ${formatRowIdentity(row[primaryKey])}` : `Item ${index + 1}`}
                      </strong>
                    </div>
                    {isSelected && <span className="overview-kind-badge">selected</span>}
                  </div>

                  <div className="result-card-grid">
                    {visibleColumns.map((columnName) => {
                      const resourceColumn = resource.columns.find((column) => column.name === columnName);
                      return (
                        <div key={columnName} className="result-cell">
                          <span className="result-cell-label">{columnName}</span>
                          <div className={`cell-content ${resourceColumn?.relation ? 'is-relation' : ''}`}>
                            {renderCellValue(row[columnName])}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="overview-empty">No rows match the current query.</p>
        )}
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

function formatSortIndicator(
  sortEntry: OverviewUrlState['sorting'][number] | null,
  sortIndex: number
) {
  if (!sortEntry) {
    return 'sort';
  }
  const direction = sortEntry.desc ? '↓' : '↑';
  return sortIndex > 0 ? `${direction} ${sortIndex + 1}` : direction;
}

function formatRowIdentity(value: unknown) {
  if (value === null || value === undefined) {
    return 'null';
  }
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  return formatJson(value);
}

function buildRowKey(row: Record<string, unknown>, index: number, primaryKey: string | null) {
  if (primaryKey) {
    const primaryValue = row[primaryKey];
    if (typeof primaryValue === 'string' || typeof primaryValue === 'number') {
      return `${primaryKey}:${primaryValue}`;
    }
  }
  return `row:${index}`;
}

function handleRowKeyDown(
  event: KeyboardEvent<HTMLDivElement>,
  row: Record<string, unknown>,
  onRowSelect: (row: Record<string, unknown>) => void
) {
  if (event.key !== 'Enter' && event.key !== ' ') {
    return;
  }
  event.preventDefault();
  onRowSelect(row);
}

function ControlBar({
  fields,
  filters,
  relationColumns,
  state,
  columns,
  onStateChange
}: {
  fields: string[];
  filters: FilterDescriptor[];
  relationColumns: ResourceOverview['columns'];
  state: OverviewUrlState;
  columns: Array<{ id: string; visible: boolean; onToggle: (event: unknown) => void }>;
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
        <label className="grid gap-1.5 text-sm text-stoneink-800">
          <span className="section-title">Page size</span>
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

        <details className="column-picker">
          <summary>Visible columns</summary>
          <div className="column-picker-grid">
            {columns.map((column) => (
              <label key={column.id} className="column-toggle">
                <input type="checkbox" checked={column.visible} onChange={column.onToggle} />
                <span>{column.id}</span>
              </label>
            ))}
          </div>
        </details>
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
          <p className="overview-copy">Narrow the result set with the same query operators exposed by REST.</p>
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
              aria-label={`Remove filter row on ${filter.field}`}
            >
              Remove
            </button>
          </div>
        ))}
        {filters.length === 0 && <p className="overview-empty">No filters yet.</p>}
      </div>

      <details className="filter-help">
        <summary>Operator help</summary>
        <p className="overview-copy">
          Advanced operators still compile to the server&apos;s native `field:operator=value` query syntax.
        </p>
      </details>
    </div>
  );
}
