import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { DataExplorerPanel, ExplorerHeader, QuerySummaryBar, ResourceSidebar } from './explorer';
import type { OverviewUrlState, ResourceOverview, ResourceResponse } from '../types';

const tableResource: ResourceOverview = {
  name: 'members',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'name', 'team_id'],
  row_samples: [{ id: 1, name: 'Ada', team_id: 7 }],
  columns: [
    { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true },
    { name: 'name', column_type: 'string', nullable: false, relation: null, is_primary_key: false },
    { name: 'team_id', column_type: 'integer', nullable: true, relation: 'foreign', is_primary_key: false }
  ],
  outgoing_relations: [],
  incoming_relations: [],
  many_to_many_relations: [],
  sample_item_id: '1',
  query_capabilities: {
    filter: true,
    sort: true,
    pagination: true,
    embed: true,
    item_route: true
  },
  mutation_capabilities: {
    create_item: true,
    update_item: true,
    delete_item: true,
    replace_object: false,
    patch_object: false
  }
};

const objectResource: ResourceOverview = {
  ...tableResource,
  name: 'settings',
  kind: 'object',
  row_count: null,
  key_count: 2,
  primary_key: null,
  sample_item_id: null,
  mutation_capabilities: {
    create_item: false,
    update_item: false,
    delete_item: false,
    replace_object: true,
    patch_object: true
  }
};

const baseState: OverviewUrlState = {
  mode: 'data',
  resource: 'members',
  view: 'explore',
  page: 1,
  perPage: 25,
  filters: [],
  sorting: [],
  embeds: []
};

const response: ResourceResponse = {
  status: 200,
  statusText: 'OK',
  url: 'http://localhost/members',
  rawText: '[{"id":1,"name":"Ada","team_id":7}]',
  parsed: [{ id: 1, name: 'Ada', team_id: 7 }]
};

describe('ResourceSidebar', () => {
  it('renders loading placeholders and empty search results', () => {
    const { rerender, container } = render(
      <ResourceSidebar
        groupedResources={{ table: [], object: [], value: [] }}
        loading
        search=""
        selectedResourceName={null}
        searchNeedle=""
        mobileOpen={false}
        onSearchChange={vi.fn()}
        onSelectResource={vi.fn()}
      />
    );

    expect(container.querySelectorAll('.resource-list-item.is-skeleton')).toHaveLength(6);

    rerender(
      <ResourceSidebar
        groupedResources={{ table: [], object: [], value: [] }}
        loading={false}
        search="missing"
        selectedResourceName={null}
        searchNeedle="missing"
        mobileOpen={false}
        onSearchChange={vi.fn()}
        onSelectResource={vi.fn()}
      />
    );

    expect(screen.getByText('No resources match the current search.')).toBeInTheDocument();
  });
});

describe('ExplorerHeader', () => {
  it('renders route links, toggles views, and triggers mutation actions', () => {
    const onChangeView = vi.fn();
    const onOpenCreate = vi.fn();
    const onOpenEdit = vi.fn();
    const onOpenDelete = vi.fn();

    render(
      <ExplorerHeader
        resource={tableResource}
        selectedRow={{ id: 1, name: 'Ada' }}
        readonly={false}
        view="explore"
        actions={{ createRow: true, editRow: true, deleteRow: true, editObject: false }}
        onChangeView={onChangeView}
        onOpenCreate={onOpenCreate}
        onOpenEdit={onOpenEdit}
        onOpenDelete={onOpenDelete}
      />
    );

    expect(screen.getByRole('link', { name: 'Collection' })).toHaveAttribute('href', '/members');
    expect(screen.getByRole('link', { name: 'Sample item' })).toHaveAttribute('href', '/members/1');
    expect(screen.getByRole('link', { name: 'Selected item' })).toHaveAttribute('href', '/members/1');

    fireEvent.click(screen.getByRole('button', { name: 'Raw JSON' }));
    fireEvent.click(screen.getByRole('button', { name: 'New row' }));
    fireEvent.click(screen.getByRole('button', { name: 'Edit row' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete row' }));

    expect(onChangeView).toHaveBeenCalledWith('raw');
    expect(onOpenCreate).toHaveBeenCalled();
    expect(onOpenEdit).toHaveBeenCalled();
    expect(onOpenDelete).toHaveBeenCalled();
  });
});

describe('QuerySummaryBar', () => {
  it('lets users clear all active query state', () => {
    const onClear = vi.fn();
    render(
      <QuerySummaryBar
        hasState
        onClear={onClear}
        onRemoveChip={vi.fn()}
        chips={[
          {
            id: 'sort:name',
            kind: 'sort',
            label: 'name',
            value: 'ascending',
            removeLabel: 'Remove sort on name'
          }
        ]}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Clear all' }));
    expect(onClear).toHaveBeenCalled();
  });
});

describe('DataExplorerPanel', () => {
  it('renders loading and error states', () => {
    const { rerender, container } = render(
      <DataExplorerPanel
        resource={tableResource}
        response={undefined}
        error={null}
        isLoading
        state={baseState}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={vi.fn()}
        onRowSelect={vi.fn()}
      />
    );

    expect(container.querySelector('.table-skeleton')).toBeInTheDocument();

    rerender(
      <DataExplorerPanel
        resource={tableResource}
        response={undefined}
        error={new Error('boom')}
        isLoading={false}
        state={baseState}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={vi.fn()}
        onRowSelect={vi.fn()}
      />
    );

    expect(screen.getByText('Request failed')).toBeInTheDocument();
    expect(screen.getByText('boom')).toBeInTheDocument();
  });

  it('renders raw and non-table views', () => {
    const { rerender, container } = render(
      <DataExplorerPanel
        resource={tableResource}
        response={response}
        error={null}
        isLoading={false}
        state={baseState}
        selectedRow={null}
        rawMode
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={vi.fn()}
        onRowSelect={vi.fn()}
      />
    );

    expect(container.querySelector('.json-viewer')?.textContent).toContain('"team_id": 7');
    expect(screen.getByText('Visible columns')).toBeInTheDocument();

    rerender(
      <DataExplorerPanel
        resource={objectResource}
        response={{
          status: 200,
          statusText: 'OK',
          url: 'http://localhost/settings',
          rawText: '{"theme":"warm"}',
          parsed: { theme: 'warm' }
        }}
        error={null}
        isLoading={false}
        state={{ ...baseState, resource: 'settings' }}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={vi.fn()}
        onRowSelect={vi.fn()}
      />
    );

    expect(screen.getByTestId('non-table-view')).toBeInTheDocument();
    expect(screen.getByText('This resource is JSON-first, so the raw document is the primary view.')).toBeInTheDocument();
  });

  it('updates filters, embeds, visibility, and row selection', () => {
    let latestState: OverviewUrlState = {
      ...baseState,
      filters: [{ id: 'name:eq:0', field: 'name', operator: 'eq' as const, value: 'Ada' }]
    };
    const onStateChange = vi.fn((updater: (state: OverviewUrlState) => OverviewUrlState) => {
      latestState = updater(latestState);
    });
    const onColumnVisibilityChange = vi.fn();
    const onRowSelect = vi.fn();

    render(
      <DataExplorerPanel
        resource={tableResource}
        response={{
          ...response,
          parsed: {
            first: 1,
            prev: null,
            next: 2,
            last: 2,
            page: 1,
            pages: 2,
            items: 2,
            data: [
              { id: 1, name: 'Ada', team_id: 7 },
              { id: 2, name: 'Grace', team_id: 3 }
            ]
          }
        }}
        error={null}
        isLoading={false}
        state={latestState}
        selectedRow={{ id: 1, name: 'Ada', team_id: 7 }}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={onColumnVisibilityChange}
        onStateChange={onStateChange}
        onRowSelect={onRowSelect}
      />
    );

    expect(screen.getByText('Page 1 of 2')).toBeInTheDocument();
    expect(screen.getByText('2 items')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Add filter' }));
    fireEvent.click(screen.getByLabelText(/team_id -> foreign/i));
    fireEvent.click(screen.getByText('Visible columns'));
    fireEvent.click(screen.getByLabelText('name'));
    fireEvent.keyDown(screen.getByRole('button', { name: /id: 1/i }), { key: 'Enter' });

    expect(latestState.filters.length).toBe(2);
    expect(latestState.embeds).toEqual(['team_id']);

    expect(onColumnVisibilityChange).toHaveBeenCalledWith('members', expect.any(Function));
    const visibilityUpdater = onColumnVisibilityChange.mock.calls[0]?.[1];
    expect(visibilityUpdater({ name: true })).toEqual({ name: false });
    expect(onRowSelect).toHaveBeenCalledWith({ id: 1, name: 'Ada', team_id: 7 });
  });

  it('updates page size from the control bar', () => {
    let latestState = baseState;
    const onStateChange = vi.fn((updater: (state: OverviewUrlState) => OverviewUrlState) => {
      latestState = updater(latestState);
    });
    const { container } = render(
      <DataExplorerPanel
        resource={tableResource}
        response={response}
        error={null}
        isLoading={false}
        state={baseState}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={onStateChange}
        onRowSelect={vi.fn()}
      />
    );

    fireEvent.change(container.querySelector('.secondary-controls .overview-select')!, {
      target: { value: '50' }
    });

    expect(latestState.perPage).toBe(50);
  });

  it('shows a message when all columns are hidden', () => {
    render(
      <DataExplorerPanel
        resource={tableResource}
        response={response}
        error={null}
        isLoading={false}
        state={baseState}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{ id: false, name: false, team_id: false }}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={vi.fn()}
        onRowSelect={vi.fn()}
      />
    );

    expect(screen.getByText('No visible columns. Use the column picker to show fields again.')).toBeInTheDocument();
  });
});
