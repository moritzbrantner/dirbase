import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { InspectorPanel, MutationDialog } from './overview/inspector';
import { DataExplorerPanel, QuerySummaryBar, ResourceSidebar } from './overview/explorer';
import type { OverviewUrlState, ResourceOverview, ResourceResponse } from './types';

const tableResource: ResourceOverview = {
  name: 'members',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'name', 'team_id'],
  row_samples: [{ id: 1, name: 'Ada' }],
  columns: [
    { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true },
    { name: 'name', column_type: 'string', nullable: false, relation: null, is_primary_key: false }
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

describe('MutationDialog', () => {
  it('surfaces invalid JSON and disables submission', () => {
    const onSubmit = vi.fn();
    render(
      <MutationDialog
        open
        mode="create"
        resource={tableResource}
        selectedRow={null}
        objectValue={null}
        onClose={vi.fn()}
        onSubmit={onSubmit}
      />
    );

    fireEvent.change(screen.getByRole('textbox'), { target: { value: '{' } });

    expect(screen.getByText(/JSON Parse error|Invalid JSON/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stage change' })).toBeDisabled();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});

describe('InspectorPanel', () => {
  it('renders request controls in read-only mode', () => {
    render(
      <InspectorPanel
        resource={tableResource}
        response={undefined}
        selectedRow={null}
        selectedTab="request"
        outgoingRelations={[]}
        incomingRelations={[]}
        readonly
        mobileOpen={false}
        requestPath="/members"
        requestUrl="http://localhost/members"
        onTabChange={vi.fn()}
        onCopy={vi.fn()}
        onOpenRequest={vi.fn()}
        onDrilldownOutgoing={vi.fn()}
        onDrilldownIncoming={vi.fn()}
      />
    );

    expect(screen.getByText('Read-only')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Copy URL' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Open request' })).toBeEnabled();
  });
});

describe('QuerySummaryBar', () => {
  it('renders chips and delegates chip removal', () => {
    const onRemoveChip = vi.fn();
    render(
      <QuerySummaryBar
        hasState
        onClear={vi.fn()}
        onRemoveChip={onRemoveChip}
        chips={[
          {
            id: 'name:eq:1',
            kind: 'filter',
            label: 'name',
            value: 'equals Ada',
            removeLabel: 'Remove filter on name'
          }
        ]}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Remove filter on name' }));
    expect(onRemoveChip).toHaveBeenCalledWith({
      id: 'name:eq:1',
      kind: 'filter',
      label: 'name',
      value: 'equals Ada',
      removeLabel: 'Remove filter on name'
    });
  });

  it('renders nothing when there is no active query state', () => {
    const { container } = render(
      <QuerySummaryBar hasState={false} onClear={vi.fn()} onRemoveChip={vi.fn()} chips={[]} />
    );

    expect(container).toBeEmptyDOMElement();
  });
});

describe('ResourceSidebar', () => {
  it('groups resources and exposes search changes', () => {
    const onSearchChange = vi.fn();
    const onSelectResource = vi.fn();
    render(
      <ResourceSidebar
        groupedResources={{
          table: [tableResource],
          object: [{ ...tableResource, name: 'settings', kind: 'object', row_count: null, key_count: 2 }],
          value: []
        }}
        loading={false}
        search=""
        selectedResourceName="members"
        searchNeedle=""
        mobileOpen={false}
        onSearchChange={onSearchChange}
        onSelectResource={onSelectResource}
      />
    );

    fireEvent.change(screen.getByLabelText('Search resources'), { target: { value: 'team' } });
    fireEvent.click(screen.getByRole('button', { name: /settings/i }));

    expect(screen.getAllByText('table').length).toBeGreaterThan(0);
    expect(screen.getAllByText('object').length).toBeGreaterThan(0);
    expect(onSearchChange).toHaveBeenCalledWith('team');
    expect(onSelectResource).toHaveBeenCalledWith('settings');
  });
});

describe('DataExplorerPanel', () => {
  it('renders row cards instead of a table and preserves multi-column sorting', () => {
    const onStateChange = vi.fn();
    const state: OverviewUrlState = {
      mode: 'data',
      resource: 'members',
      view: 'explore',
      page: 1,
      perPage: 25,
      filters: [],
      sorting: [{ id: 'name', desc: false }],
      embeds: []
    };
    const response: ResourceResponse = {
      status: 200,
      statusText: 'OK',
      url: 'http://localhost/members',
      rawText: '[{"id":1,"name":"Ada"},{"id":2,"name":"Grace"}]',
      parsed: [
        { id: 1, name: 'Ada', team_id: 7 },
        { id: 2, name: 'Grace', team_id: 3 }
      ]
    };

    render(
      <DataExplorerPanel
        resource={tableResource}
        response={response}
        error={null}
        isLoading={false}
        state={state}
        selectedRow={null}
        rawMode={false}
        columnVisibility={{}}
        onColumnVisibilityChange={vi.fn()}
        onStateChange={onStateChange}
        onRowSelect={vi.fn()}
      />
    );

    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(screen.getByText(/Shift-click any additional column/i)).toBeInTheDocument();

    const sortGroup = screen.getByRole('group', { name: 'Sort rows' });
    expect(within(sortGroup).getByText('↑')).toBeInTheDocument();

    fireEvent.click(within(sortGroup).getByRole('button', { name: /team_id/i }), {
      shiftKey: true
    });

    const updater = onStateChange.mock.calls[0]?.[0] as
      | ((current: OverviewUrlState) => OverviewUrlState)
      | undefined;
    expect(updater).toBeTypeOf('function');
    expect(updater?.(state).sorting).toEqual([
      { id: 'name', desc: false },
      { id: 'team_id', desc: false }
    ]);
  });
});
