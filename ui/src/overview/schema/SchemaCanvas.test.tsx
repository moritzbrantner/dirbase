import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SchemaCanvas } from './SchemaCanvas';
import type { ResourceOverview, SchemaTable } from '../../types';

const resources: ResourceOverview[] = [
  {
    name: 'users',
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: 'id',
    field_names: ['id', 'team_id', 'name'],
    row_samples: [],
    columns: [
      { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true },
      { name: 'team_id', column_type: 'integer', nullable: true, relation: 'foreign', is_primary_key: false },
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
  },
  {
    name: 'teams',
    kind: 'table',
    row_count: 1,
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
  }
];

const effectiveTables: Record<string, SchemaTable> = {
  users: {
    kind: 'object',
    primary_key: 'id',
    columns: {
      id: { column_type: 'integer', nullable: false },
      team_id: { column_type: 'integer', nullable: true },
      name: { column_type: 'string', nullable: false }
    },
    foreign_keys: {
      team_id: {
        target_table: 'teams',
        target_column: 'id'
      }
    }
  },
  teams: {
    kind: 'object',
    primary_key: 'id',
    columns: {
      id: { column_type: 'integer', nullable: false },
      name: { column_type: 'string', nullable: false }
    },
    foreign_keys: {}
  }
};

describe('SchemaCanvas', () => {
  it('renders only handles that can actually participate in a connection', async () => {
    render(
      <div style={{ width: '1200px', height: '800px' }}>
        <SchemaCanvas
          resources={resources}
          effectiveTables={effectiveTables}
          selection={null}
          readonly={false}
          structuredEditingDisabled={false}
          autoArrangeNonce={0}
          onSelectTable={vi.fn()}
          onSelectColumn={vi.fn()}
          onSelectRelation={vi.fn()}
          onCreateRelationship={vi.fn()}
        />
      </div>
    );

    await waitFor(() => expect(screen.getByLabelText('Minify users')).toBeInTheDocument());

    expect(screen.getByTestId('users:id:target-handle')).toBeInTheDocument();
    expect(screen.queryByTestId('users:id:source-handle')).not.toBeInTheDocument();
    expect(screen.getByTestId('users:team_id:source-handle')).toBeInTheDocument();
    expect(screen.queryByTestId('users:team_id:target-handle')).not.toBeInTheDocument();
  });

  it('can minify and expand a node', async () => {
    render(
      <div style={{ width: '1200px', height: '800px' }}>
        <SchemaCanvas
          resources={resources}
          effectiveTables={effectiveTables}
          selection={null}
          readonly={false}
          structuredEditingDisabled={false}
          autoArrangeNonce={0}
          onSelectTable={vi.fn()}
          onSelectColumn={vi.fn()}
          onSelectRelation={vi.fn()}
          onCreateRelationship={vi.fn()}
        />
      </div>
    );

    await waitFor(() => expect(screen.getByLabelText('Minify users')).toBeInTheDocument());

    fireEvent.click(screen.getByLabelText('Minify users'));
    expect(screen.getByText('2 compatible columns hidden')).toBeInTheDocument();
    expect(screen.queryByText('team_id')).not.toBeInTheDocument();
    expect(screen.queryByTestId('users:id:target-handle')).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Expand users'));
    await waitFor(() => expect(screen.getByText('team_id')).toBeInTheDocument());
    expect(screen.getByTestId('users:id:target-handle')).toBeInTheDocument();
  });

  it('selects tables and columns from node interactions', async () => {
    const onSelectTable = vi.fn();
    const onSelectColumn = vi.fn();

    render(
      <div style={{ width: '1200px', height: '800px' }}>
        <SchemaCanvas
          resources={resources}
          effectiveTables={effectiveTables}
          selection={{ kind: 'column', tableName: 'users', columnName: 'team_id' }}
          readonly={false}
          structuredEditingDisabled={false}
          autoArrangeNonce={0}
          onSelectTable={onSelectTable}
          onSelectColumn={onSelectColumn}
          onSelectRelation={vi.fn()}
          onCreateRelationship={vi.fn()}
        />
      </div>
    );

    await waitFor(() => expect(screen.getByRole('button', { name: 'users' })).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'users' }));
    fireEvent.click(screen.getByRole('button', { name: /team_id.*fk/i }));

    expect(onSelectTable).toHaveBeenCalledWith('users');
    expect(onSelectColumn).toHaveBeenCalledWith('users', 'team_id');
  });

  it('shows an empty-state message when no compatible columns are available', async () => {
    render(
      <div style={{ width: '1200px', height: '800px' }}>
        <SchemaCanvas
          resources={[
            {
              ...resources[0],
              name: 'notes',
              primary_key: null,
              field_names: ['title'],
              columns: [
                {
                  name: 'title',
                  column_type: 'string',
                  nullable: false,
                  relation: null,
                  is_primary_key: false
                }
              ]
            }
          ]}
          effectiveTables={{
            notes: {
              kind: 'object',
              columns: {
                title: { column_type: 'string', nullable: false }
              },
              foreign_keys: {}
            }
          }}
          selection={null}
          readonly={false}
          structuredEditingDisabled={false}
          autoArrangeNonce={0}
          onSelectTable={vi.fn()}
          onSelectColumn={vi.fn()}
          onSelectRelation={vi.fn()}
          onCreateRelationship={vi.fn()}
        />
      </div>
    );

    await waitFor(() =>
      expect(screen.getByText('No compatible key columns are available in this table.')).toBeInTheDocument()
    );
  });
});
