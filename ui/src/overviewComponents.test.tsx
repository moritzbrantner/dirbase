import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { InspectorPanel, MutationDialog } from './overview/inspector';
import { QuerySummaryBar, ResourceSidebar } from './overview/explorer';
import type { ResourceOverview } from './types';

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
    expect(screen.getByRole('button', { name: 'Submit request' })).toBeDisabled();
    expect(onSubmit).not.toHaveBeenCalled();
  });
});

describe('InspectorPanel schema editor', () => {
  it('disables schema write controls in read-only mode', () => {
    render(
      <InspectorPanel
        resource={tableResource}
        response={undefined}
        schemaDraft='{"tables":{}}'
        schemaStatus={null}
        schemaDiffSummary={[]}
        schemaValidationError={null}
        selectedRow={null}
        selectedTab="schema"
        outgoingRelations={[]}
        incomingRelations={[]}
        readonly
        mobileOpen={false}
        schemaBusy={false}
        canSaveSchema
        canInferSchema
        requestPath="/members"
        requestUrl="http://localhost/members"
        onTabChange={vi.fn()}
        onSchemaDraftChange={vi.fn()}
        onCopy={vi.fn()}
        onOpenRequest={vi.fn()}
        onReloadSchema={vi.fn()}
        onSaveSchema={vi.fn()}
        onInferSchema={vi.fn()}
        onDrilldownOutgoing={vi.fn()}
        onDrilldownIncoming={vi.fn()}
      />
    );

    expect(screen.getByTestId('schema-editor')).toHaveAttribute('readonly');
    expect(screen.getByRole('button', { name: 'Infer from data' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Reload' })).toBeEnabled();
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
