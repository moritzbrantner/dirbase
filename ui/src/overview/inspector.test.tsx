import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { InspectorPanel, MutationDialog } from './inspector';
import type { ResourceOverview, ResourceResponse } from '../types';

const tableResource: ResourceOverview = {
  name: 'members',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'name'],
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

describe('InspectorPanel', () => {
  it('copies request details and renders paginated metadata', async () => {
    const onCopy = vi.fn().mockResolvedValue(undefined);
    const onOpenRequest = vi.fn();
    const response: ResourceResponse = {
      status: 200,
      statusText: 'OK',
      url: 'http://localhost/members?page=2',
      rawText: '',
      parsed: {
        first: 1,
        prev: 1,
        next: 3,
        last: 4,
        page: 2,
        pages: 4,
        items: 12,
        data: [{ id: 1, name: 'Ada' }]
      }
    };

    render(
      <InspectorPanel
        resource={tableResource}
        response={response}
        selectedRow={null}
        selectedTab="request"
        outgoingRelations={[]}
        incomingRelations={[]}
        readonly={false}
        mobileOpen={false}
        requestPath="/members?page=2"
        requestUrl="http://localhost/members?page=2"
        onTabChange={vi.fn()}
        onCopy={onCopy}
        onOpenRequest={onOpenRequest}
        onDrilldownOutgoing={vi.fn()}
        onDrilldownIncoming={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Copy URL' }));
    fireEvent.click(screen.getByRole('button', { name: 'Copy curl' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open request' }));

    await waitFor(() => {
      expect(onCopy).toHaveBeenNthCalledWith(1, '/members?page=2');
      expect(onCopy).toHaveBeenNthCalledWith(
        2,
        "curl -H 'Accept: application/json' 'http://localhost/members?page=2'"
      );
    });
    expect(onOpenRequest).toHaveBeenCalled();
    expect(screen.getByText('Page 2')).toBeInTheDocument();
    expect(screen.getByText('12 items')).toBeInTheDocument();
    expect(screen.getByText('4 pages')).toBeInTheDocument();
  });

  it('renders object-resource selection snapshots', () => {
    const { container } = render(
      <InspectorPanel
        resource={objectResource}
        response={{
          status: 200,
          statusText: 'OK',
          url: 'http://localhost/settings',
          rawText: '{"theme":"warm"}',
          parsed: { theme: 'warm' }
        }}
        selectedRow={null}
        selectedTab="selection"
        outgoingRelations={[]}
        incomingRelations={[]}
        readonly={false}
        mobileOpen={false}
        requestPath="/settings"
        requestUrl="http://localhost/settings"
        onTabChange={vi.fn()}
        onCopy={vi.fn()}
        onOpenRequest={vi.fn()}
        onDrilldownOutgoing={vi.fn()}
        onDrilldownIncoming={vi.fn()}
      />
    );

    expect(screen.getByText('Snapshot')).toBeInTheDocument();
    expect(screen.getByText('{ theme }')).toBeInTheDocument();
    expect(container.querySelector('.json-viewer')?.textContent).toContain('"name": "Ada"');
    expect(screen.queryByText('Selected row')).not.toBeInTheDocument();
  });
});

describe('MutationDialog', () => {
  it('builds and submits PATCH requests for changed rows', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    render(
      <MutationDialog
        open
        mode="edit"
        resource={tableResource}
        selectedRow={{ id: 1, name: 'Ada' }}
        objectValue={null}
        onClose={onClose}
        onSubmit={onSubmit}
      />
    );

    expect(screen.getByText('No changed keys detected. Edit the JSON or switch to full replace.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'PATCH request' })).toBeDisabled();

    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: '{\n  "id": 1,\n  "name": "Grace"\n}' }
    });
    fireEvent.click(screen.getByRole('button', { name: 'PATCH request' }));

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({
        method: 'PATCH',
        path: '/members/1',
        body: '{"name":"Grace"}',
        changedKeys: ['name'],
        requiresConfirmation: false
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('requires confirmation before submitting PUT replacements', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);

    render(
      <MutationDialog
        open
        mode="edit"
        resource={tableResource}
        selectedRow={{ id: 1, name: 'Ada' }}
        objectValue={null}
        onClose={vi.fn()}
        onSubmit={onSubmit}
      />
    );

    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: '{\n  "id": 1,\n  "name": "Grace"\n}' }
    });
    fireEvent.click(
      screen.getByLabelText(
        'Use `PUT` to replace the full document instead of sending only changed keys.'
      )
    );

    expect(screen.getByRole('button', { name: 'PUT request' })).toBeDisabled();
    fireEvent.click(screen.getByLabelText('I understand this will replace the full document with PUT.'));
    fireEvent.click(screen.getByRole('button', { name: 'PUT request' }));

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({
        method: 'PUT',
        path: '/members/1',
        body: '{"id":1,"name":"Grace"}',
        changedKeys: ['name'],
        requiresConfirmation: true
      })
    );
  });

  it('requires confirmation for deletes and surfaces submit failures', async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error('Delete failed.'));
    const onClose = vi.fn();

    render(
      <MutationDialog
        open
        mode="delete"
        resource={tableResource}
        selectedRow={{ id: 1, name: 'Ada' }}
        objectValue={null}
        onClose={onClose}
        onSubmit={onSubmit}
      />
    );

    const submit = screen.getByRole('button', { name: 'DELETE request' });
    expect(submit).toBeDisabled();

    fireEvent.click(screen.getByLabelText('I understand this delete cannot be undone.'));
    fireEvent.click(submit);

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({
        method: 'DELETE',
        path: '/members/1',
        body: null,
        changedKeys: [],
        requiresConfirmation: true
      })
    );
    expect(await screen.findByText('Delete failed.')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });
});
