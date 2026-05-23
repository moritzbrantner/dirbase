import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SchemaDetailsPanel } from './SchemaDetailsPanel';
import type {
  DeclaredSchemaResponse,
  SchemaColumn,
  SchemaColumnOverrideInput,
  SchemaResponse,
  SchemaWorkspaceSelection
} from '../../types';

const baseTable = {
  kind: 'object',
  primary_key: 'id',
  columns: {
    id: { column_type: 'integer', nullable: false },
    name: { column_type: 'string', nullable: true }
  },
  foreign_keys: {}
};

function renderPanel({
  selection = { kind: 'column', tableName: 'users', columnName: 'name' },
  column = { column_type: 'string', nullable: true },
  declaredDraft = { tables: {} },
  onSetColumnOverride = vi.fn(),
  onSetUniqueConstraints = vi.fn()
}: {
  selection?: SchemaWorkspaceSelection;
  column?: SchemaColumn;
  declaredDraft?: DeclaredSchemaResponse;
  onSetColumnOverride?: (
    tableName: string,
    columnName: string,
    next: SchemaColumnOverrideInput
  ) => void;
  onSetUniqueConstraints?: (tableName: string, unique: string[][]) => void;
} = {}) {
  const effectiveSchema: SchemaResponse = {
    tables: {
      users: {
        ...baseTable,
        columns: {
          ...baseTable.columns,
          name: column
        }
      }
    }
  };

  render(
    <SchemaDetailsPanel
      selection={selection}
      discoveredTables={['users']}
      effectiveSchema={effectiveSchema}
      inferredSchema={effectiveSchema}
      declaredDraft={declaredDraft}
      readonly={false}
      mobileOpen={false}
      structuredEditingDisabled={false}
      onSelectRelation={vi.fn()}
      onSetTableKind={vi.fn()}
      onSetPrimaryKey={vi.fn()}
      onSetColumnOverride={onSetColumnOverride}
      onSetUniqueConstraints={onSetUniqueConstraints}
      onUpdateRelation={vi.fn()}
      onRemoveRelation={vi.fn()}
      onResetRelation={vi.fn()}
    />
  );
}

describe('SchemaDetailsPanel', () => {
  it('parses string column enum, bounds, pattern, and nullability edits', () => {
    const onSetColumnOverride = vi.fn();
    renderPanel({
      column: { column_type: 'string', nullable: true },
      onSetColumnOverride
    });

    fireEvent.change(screen.getByLabelText('Nullability'), { target: { value: 'required' } });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: false
    });

    fireEvent.change(screen.getByLabelText('Nullability'), { target: { value: 'automatic' } });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: null
    });

    fireEvent.change(screen.getByLabelText('Enum values'), {
      target: { value: 'Ada, Grace,  ' }
    });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: true,
      enumValues: ['Ada', 'Grace']
    });

    fireEvent.change(screen.getByLabelText('Min length'), { target: { value: '2' } });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: true,
      minLength: 2
    });

    fireEvent.change(screen.getByLabelText('Max length'), { target: { value: '9' } });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: true,
      maxLength: 9
    });

    fireEvent.change(screen.getByLabelText('Pattern'), { target: { value: '^A' } });
    expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
      columnType: 'string',
      nullable: true,
      pattern: '^A'
    });
  });

  it('converts integer min and max bounds to numbers', () => {
    expectNumericBounds('integer');
  });

  it('converts float min and max bounds to numbers', () => {
    expectNumericBounds('float');
  });

  it('converts big_integer min and max bounds to numbers', () => {
    expectNumericBounds('big_integer');
  });

  it('converts decimal min and max bounds to numbers', () => {
    expectNumericBounds('decimal');
  });

  it('keeps date min and max bounds as strings', () => {
    expectStringBounds('date');
  });

  it('keeps datetime min and max bounds as strings', () => {
    expectStringBounds('datetime');
  });

  it('shows string-backed validation controls for uuid columns', () => {
    expectStringBackedControls('uuid');
  });

  it('shows string-backed validation controls for big_integer columns', () => {
    expectStringBackedControls('big_integer');
  });

  it('shows string-backed validation controls for decimal columns', () => {
    expectStringBackedControls('decimal');
  });

  it('parses table unique constraints from comma and newline separated input', () => {
    const onSetUniqueConstraints = vi.fn();
    renderPanel({
      selection: { kind: 'table', tableName: 'users' },
      declaredDraft: { tables: { users: { unique: [['id']] } } },
      onSetUniqueConstraints
    });

    fireEvent.change(screen.getByLabelText('Unique constraints'), {
      target: { value: 'id, name\n\nemail,  ' }
    });

    expect(onSetUniqueConstraints).toHaveBeenCalledWith('users', [
      ['id', 'name'],
      ['email']
    ]);
  });
});

function expectNumericBounds(columnType: string) {
  const onSetColumnOverride = vi.fn();
  renderPanel({
    column: { column_type: columnType, nullable: false },
    onSetColumnOverride
  });

  fireEvent.change(screen.getByLabelText('Min'), { target: { value: '12.5' } });
  expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
    columnType,
    nullable: false,
    min: 12.5
  });

  fireEvent.change(screen.getByLabelText('Max'), { target: { value: '20' } });
  expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
    columnType,
    nullable: false,
    max: 20
  });
}

function expectStringBounds(columnType: string) {
  const onSetColumnOverride = vi.fn();
  renderPanel({
    column: { column_type: columnType, nullable: true },
    onSetColumnOverride
  });

  fireEvent.change(screen.getByLabelText('Min'), { target: { value: '2026-05-23' } });
  expect(onSetColumnOverride).toHaveBeenLastCalledWith('users', 'name', {
    columnType,
    nullable: true,
    min: '2026-05-23'
  });
}

function expectStringBackedControls(columnType: string) {
  renderPanel({ column: { column_type: columnType, nullable: true } });

  expect(screen.getByLabelText('Min length')).toBeInTheDocument();
  expect(screen.getByLabelText('Max length')).toBeInTheDocument();
  expect(screen.getByLabelText('Pattern')).toBeInTheDocument();
}
