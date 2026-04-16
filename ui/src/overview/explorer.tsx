import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
  type Updater,
  type VisibilityState
} from '@tanstack/react-table';
import type { MouseEvent } from 'react';

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
import { HighlightText, TableSkeleton, renderCapabilityChip, renderCellValue } from './shared';

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
              aria-label={`Remove filter row on ${filter.field}`}
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
