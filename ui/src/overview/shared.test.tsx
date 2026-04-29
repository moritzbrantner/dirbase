import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { HighlightText, TableSkeleton, ToastViewport, getJsonValidationError, groupResources, renderCellValue, renderLiveUpdateLabel } from './shared';
import type { ResourceOverview } from '../types';

const tableResource: ResourceOverview = {
  name: 'members',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'name'],
  row_samples: [],
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

describe('shared overview helpers', () => {
  it('renders toast messages and a table skeleton', () => {
    const { container } = render(
      <>
        <ToastViewport
          toasts={[
            { id: 1, tone: 'info', message: 'Refreshing' },
            { id: 2, tone: 'error', message: 'Paused' }
          ]}
        />
        <TableSkeleton />
      </>
    );

    expect(screen.getByText('Refreshing')).toHaveClass('toast-message', 'is-info');
    expect(screen.getByText('Paused')).toHaveClass('toast-message', 'is-error');
    expect(container.querySelectorAll('.table-skeleton-row')).toHaveLength(6);
    expect(container.querySelectorAll('.skeleton-cell')).toHaveLength(30);
  });

  it('highlights matching text and leaves unmatched text unchanged', () => {
    const { rerender } = render(<HighlightText text="Member Team" needle="team" />);
    expect(screen.getByText('Team').tagName).toBe('MARK');

    rerender(<HighlightText text="Member Team" needle="owner" />);
    expect(screen.queryByText('Team')?.tagName).not.toBe('MARK');

    rerender(<HighlightText text="Member Team" needle="   " />);
    expect(screen.getByText('Member Team')).toBeInTheDocument();
  });

  it('renders scalar, null, array, and object cell values', () => {
    const { rerender, container } = render(<div>{renderCellValue('Ada')}</div>);
    expect(screen.getByText('Ada')).toBeInTheDocument();

    rerender(<div>{renderCellValue(null)}</div>);
    expect(screen.getByText('null')).toHaveClass('cell-muted');

    rerender(<div>{renderCellValue([1, 2, 3])}</div>);
    expect(screen.getByText('Array(3)')).toBeInTheDocument();
    expect(container.querySelector('.json-viewer')?.textContent).toContain('1');

    rerender(<div>{renderCellValue({ id: 1, name: 'Ada' })}</div>);
    expect(screen.getByText('{ id, name }')).toBeInTheDocument();
    expect(container.querySelector('.json-viewer')?.textContent).toContain('"name": "Ada"');
  });

  it('groups resources and formats live-update and JSON validation output', () => {
    const grouped = groupResources([
      tableResource,
      { ...tableResource, name: 'settings', kind: 'object', row_count: null, key_count: 2 },
      { ...tableResource, name: 'build', kind: 'value', row_count: null, key_count: null }
    ]);

    expect(grouped.table.map((resource) => resource.name)).toEqual(['members']);
    expect(grouped.object.map((resource) => resource.name)).toEqual(['settings']);
    expect(grouped.value.map((resource) => resource.name)).toEqual(['build']);

    expect(renderLiveUpdateLabel('live')).toBe('Live');
    expect(renderLiveUpdateLabel('reconnecting')).toBe('Reconnecting');
    expect(renderLiveUpdateLabel('paused')).toBe('Paused');
    expect(renderLiveUpdateLabel('connecting')).toBe('Connecting');

    expect(getJsonValidationError('{"ok":true}')).toBeNull();
    expect(getJsonValidationError('{')).toMatch(/Expected|JSON|property/i);
  });
});
