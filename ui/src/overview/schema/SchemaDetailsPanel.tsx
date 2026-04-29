import { useEffect, useState } from 'react';

import {
  getRelationOrigin,
  isColumnOverridden,
  isSchemaTableManual,
  normalizeSchema
} from '../../schemaWorkspace';
import type {
  DeclaredSchemaResponse,
  SchemaColumn,
  SchemaColumnOverrideInput,
  SchemaResponse,
  SchemaWorkspaceSelection
} from '../../types';

export function SchemaDetailsPanel({
  selection,
  discoveredTables,
  effectiveSchema,
  inferredSchema,
  declaredDraft,
  readonly,
  mobileOpen,
  structuredEditingDisabled,
  onSelectRelation,
  onSetTableKind,
  onSetPrimaryKey,
  onSetColumnOverride,
  onSetUniqueConstraints,
  onUpdateRelation,
  onRemoveRelation,
  onResetRelation
}: {
  selection: SchemaWorkspaceSelection | null;
  discoveredTables: string[];
  effectiveSchema: SchemaResponse;
  inferredSchema: SchemaResponse;
  declaredDraft: DeclaredSchemaResponse;
  readonly: boolean;
  mobileOpen: boolean;
  structuredEditingDisabled: boolean;
  onSelectRelation: (tableName: string, sourceColumn: string) => void;
  onSetTableKind: (tableName: string, kind: string | null) => void;
  onSetPrimaryKey: (tableName: string, primaryKey: string | null) => void;
  onSetColumnOverride: (
    tableName: string,
    columnName: string,
    next: SchemaColumnOverrideInput
  ) => void;
  onSetUniqueConstraints: (tableName: string, unique: string[][]) => void;
  onUpdateRelation: (
    tableName: string,
    sourceColumn: string,
    targetTable: string,
    targetColumn: string
  ) => void;
  onRemoveRelation: (tableName: string, sourceColumn: string) => void;
  onResetRelation: (tableName: string, sourceColumn: string) => void;
}) {
  const effective = normalizeSchema(effectiveSchema);
  const inferred = normalizeSchema(inferredSchema);

  if (!selection) {
    return (
      <aside className={`schema-details shell-card ${mobileOpen ? 'mobile-sheet-open' : ''}`}>
        <div className="schema-sidebar-head">
          <div>
            <p className="section-title">Details</p>
            <h3 className="text-lg font-semibold text-stoneink-900">Select a target</h3>
          </div>
        </div>
        <p className="overview-copy">
          Select a table, a column, or a relation in the graph to edit the declared schema overlay.
        </p>
      </aside>
    );
  }

  const disabled = readonly || structuredEditingDisabled;
  const table = effective.tables?.[selection.tableName];
  if (!table) {
    return (
      <aside className={`schema-details shell-card ${mobileOpen ? 'mobile-sheet-open' : ''}`}>
        <p className="overview-empty">Selected table is no longer available.</p>
      </aside>
    );
  }

  return (
    <aside className={`schema-details shell-card ${mobileOpen ? 'mobile-sheet-open' : ''}`}>
      <div className="schema-sidebar-head">
        <div>
          <p className="section-title">Details</p>
          <h3 className="text-lg font-semibold text-stoneink-900">{selection.tableName}</h3>
        </div>
        <span className={`schema-origin-badge ${isSchemaTableManual(declaredDraft, selection.tableName) ? 'is-manual' : 'is-inferred'}`}>
          {isSchemaTableManual(declaredDraft, selection.tableName) ? 'manual' : 'inferred'}
        </span>
      </div>

      {selection.kind === 'table' && (
        <TableDetails
          tableName={selection.tableName}
          table={table}
          unique={declaredDraft.tables?.[selection.tableName]?.unique ?? []}
          disabled={disabled}
          onSelectRelation={onSelectRelation}
          onSetTableKind={onSetTableKind}
          onSetPrimaryKey={onSetPrimaryKey}
          onSetUniqueConstraints={onSetUniqueConstraints}
        />
      )}

      {selection.kind === 'column' && (
        <ColumnDetails
          tableName={selection.tableName}
          columnName={selection.columnName}
          column={table.columns?.[selection.columnName]}
          overridden={isColumnOverridden(declaredDraft, selection.tableName, selection.columnName)}
          disabled={disabled}
          onSetColumnOverride={onSetColumnOverride}
        />
      )}

      {selection.kind === 'relation' && (
        <RelationDetails
          tableName={selection.tableName}
          sourceColumn={selection.relationSourceColumn}
          target={table.foreign_keys?.[selection.relationSourceColumn] ?? null}
          targetTables={discoveredTables}
          effectiveSchema={effective}
          origin={getRelationOrigin(inferred, declaredDraft, selection.tableName, selection.relationSourceColumn)}
          disabled={disabled}
          onUpdateRelation={onUpdateRelation}
          onRemoveRelation={onRemoveRelation}
          onResetRelation={onResetRelation}
        />
      )}
    </aside>
  );
}

function TableDetails({
  tableName,
  table,
  unique,
  disabled,
  onSelectRelation,
  onSetTableKind,
  onSetPrimaryKey,
  onSetUniqueConstraints
}: {
  tableName: string;
  table: NonNullable<SchemaResponse['tables']>[string];
  unique: string[][];
  disabled: boolean;
  onSelectRelation: (tableName: string, sourceColumn: string) => void;
  onSetTableKind: (tableName: string, kind: string | null) => void;
  onSetPrimaryKey: (tableName: string, primaryKey: string | null) => void;
  onSetUniqueConstraints: (tableName: string, unique: string[][]) => void;
}) {
  const columnNames = Object.keys(table.columns ?? {}).sort();
  const relationNames = Object.keys(table.foreign_keys ?? {}).sort();
  const manyToManyNames = Object.keys(table.many_to_many ?? {}).sort();

  return (
    <div className="schema-details-stack">
      <label className="schema-field">
        <span className="schema-field-label">Kind</span>
        <select
          className="overview-select"
          value={table.kind ?? 'unknown'}
          onChange={(event) =>
            onSetTableKind(tableName, event.target.value === 'automatic' ? null : event.target.value)
          }
          disabled={disabled}
        >
          <option value="automatic">Automatic</option>
          <option value="object">Object</option>
          <option value="relation">Relation</option>
          <option value="unknown">Unknown</option>
        </select>
      </label>

      <label className="schema-field">
        <span className="schema-field-label">Primary key</span>
        <select
          className="overview-select"
          value={table.primary_key ?? 'automatic'}
          onChange={(event) =>
            onSetPrimaryKey(tableName, event.target.value === 'automatic' ? null : event.target.value)
          }
          disabled={disabled}
        >
          <option value="automatic">Automatic</option>
          {columnNames.map((columnName) => (
            <option key={columnName} value={columnName}>
              {columnName}
            </option>
          ))}
        </select>
      </label>

      <label className="schema-field">
        <span className="schema-field-label">Unique constraints</span>
        <textarea
          className="overview-input"
          value={unique.map((constraint) => constraint.join(', ')).join('\n')}
          onChange={(event) => onSetUniqueConstraints(tableName, parseUniqueConstraints(event.target.value))}
          disabled={disabled}
          rows={3}
        />
      </label>

      <div className="schema-info-card">
        <p className="section-title">Columns</p>
        <div className="resource-field-list">
          {columnNames.map((columnName) => (
            <span key={columnName} className="resource-field-pill">
              {columnName}
            </span>
          ))}
        </div>
      </div>

      {relationNames.length > 0 && (
        <div className="schema-info-card">
          <p className="section-title">Relations</p>
          <div className="schema-relation-list">
            {relationNames.map((sourceColumn) => (
              <button
                key={sourceColumn}
                type="button"
                className="relation-link-button"
                onClick={() => onSelectRelation(tableName, sourceColumn)}
                disabled={disabled}
              >
                <strong>{sourceColumn}</strong>
                <span>
                  {table.foreign_keys?.[sourceColumn]?.target_table}.{table.foreign_keys?.[sourceColumn]?.target_column}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}

      {manyToManyNames.length > 0 && (
        <div className="schema-info-card">
          <p className="section-title">Many-to-many</p>
          <div className="schema-relation-list">
            {manyToManyNames.map((relationName) => {
              const relation = table.many_to_many?.[relationName];
              if (!relation) {
                return null;
              }

              return (
                <div key={relationName} className="relation-link-button is-readonly">
                  <strong>{relationName}</strong>
                  <span>
                    via {relation.through_table} ({relation.source_column} -&gt;{' '}
                    {relation.through_target_column})
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

function ColumnDetails({
  tableName,
  columnName,
  column,
  overridden,
  disabled,
  onSetColumnOverride
}: {
  tableName: string;
  columnName: string;
  column: SchemaColumn | undefined;
  overridden: boolean;
  disabled: boolean;
  onSetColumnOverride: (
    tableName: string,
    columnName: string,
    next: SchemaColumnOverrideInput
  ) => void;
}) {
  if (!column) {
    return <p className="overview-empty">Selected column is no longer available.</p>;
  }

  return (
    <div className="schema-details-stack">
      <div className="schema-info-card">
        <p className="section-title">Column</p>
        <p className="text-sm font-semibold text-stoneink-900">{columnName}</p>
        <p className="overview-copy">
          {overridden ? 'This column has a declared override.' : 'This column is currently inferred.'}
        </p>
      </div>

      <label className="schema-field">
        <span className="schema-field-label">Type</span>
        <select
          className="overview-select"
          value={column.column_type ?? 'string'}
          onChange={(event) =>
            onSetColumnOverride(tableName, columnName, {
              columnType: event.target.value === 'automatic' ? null : event.target.value,
              nullable: column.nullable ?? true
            })
          }
          disabled={disabled}
        >
          <option value="automatic">Automatic</option>
          <option value="integer">Integer</option>
          <option value="float">Float</option>
          <option value="boolean">Boolean</option>
          <option value="string">String</option>
          <option value="json">Json</option>
          <option value="date">Date</option>
          <option value="datetime">DateTime</option>
          <option value="uuid">Uuid</option>
          <option value="big_integer">Big integer</option>
          <option value="decimal">Decimal</option>
        </select>
      </label>

      <label className="schema-field">
        <span className="schema-field-label">Nullability</span>
        <select
          className="overview-select"
          value={column.nullable === undefined ? 'automatic' : column.nullable ? 'nullable' : 'required'}
          onChange={(event) =>
            onSetColumnOverride(tableName, columnName, {
              columnType: column.column_type ?? 'string',
              nullable:
                event.target.value === 'automatic'
                  ? null
                  : event.target.value === 'nullable'
            })
          }
          disabled={disabled}
        >
          <option value="automatic">Automatic</option>
          <option value="required">Required</option>
          <option value="nullable">Nullable</option>
        </select>
      </label>

      {column.column_type === 'string' ? (
        <label className="schema-field">
          <span className="schema-field-label">Enum values</span>
          <input
            className="overview-input"
            value={column.enum_values?.join(', ') ?? ''}
            onChange={(event) =>
              onSetColumnOverride(tableName, columnName, {
                columnType: column.column_type ?? 'string',
                nullable: column.nullable ?? true,
                enumValues: event.target.value
                  .split(',')
                  .map((value) => value.trim())
                  .filter(Boolean)
              })
            }
            disabled={disabled}
          />
        </label>
      ) : null}

      {isBoundedColumnType(column.column_type ?? 'string') ? (
        <div className="schema-field-grid">
          <label className="schema-field">
            <span className="schema-field-label">Min</span>
            <input
              className="overview-input"
              type={boundInputType(column.column_type ?? 'string')}
              value={column.min ?? ''}
              onChange={(event) =>
                onSetColumnOverride(tableName, columnName, {
                  columnType: column.column_type ?? 'string',
                  nullable: column.nullable ?? true,
                  min: cleanBoundInputValue(column.column_type ?? 'string', event.target.value)
                })
              }
              disabled={disabled}
            />
          </label>
          <label className="schema-field">
            <span className="schema-field-label">Max</span>
            <input
              className="overview-input"
              type={boundInputType(column.column_type ?? 'string')}
              value={column.max ?? ''}
              onChange={(event) =>
                onSetColumnOverride(tableName, columnName, {
                  columnType: column.column_type ?? 'string',
                  nullable: column.nullable ?? true,
                  max: cleanBoundInputValue(column.column_type ?? 'string', event.target.value)
                })
              }
              disabled={disabled}
            />
          </label>
        </div>
      ) : null}

      {isStringBackedColumnType(column.column_type ?? 'string') ? (
        <>
          <div className="schema-field-grid">
            <label className="schema-field">
              <span className="schema-field-label">Min length</span>
              <input
                className="overview-input"
                type="number"
                min="0"
                value={column.min_length ?? ''}
                onChange={(event) =>
                  onSetColumnOverride(tableName, columnName, {
                    columnType: column.column_type ?? 'string',
                    nullable: column.nullable ?? true,
                    minLength: event.target.value === '' ? null : Number(event.target.value)
                  })
                }
                disabled={disabled}
              />
            </label>
            <label className="schema-field">
              <span className="schema-field-label">Max length</span>
              <input
                className="overview-input"
                type="number"
                min="0"
                value={column.max_length ?? ''}
                onChange={(event) =>
                  onSetColumnOverride(tableName, columnName, {
                    columnType: column.column_type ?? 'string',
                    nullable: column.nullable ?? true,
                    maxLength: event.target.value === '' ? null : Number(event.target.value)
                  })
                }
                disabled={disabled}
              />
            </label>
          </div>
          <label className="schema-field">
            <span className="schema-field-label">Pattern</span>
            <input
              className="overview-input"
              value={column.pattern ?? ''}
              onChange={(event) =>
                onSetColumnOverride(tableName, columnName, {
                  columnType: column.column_type ?? 'string',
                  nullable: column.nullable ?? true,
                  pattern: event.target.value || null
                })
              }
              disabled={disabled}
            />
          </label>
        </>
      ) : null}
    </div>
  );
}

function isNumericColumnType(columnType: string): boolean {
  return ['integer', 'float', 'big_integer', 'decimal'].includes(columnType);
}

function isBoundedColumnType(columnType: string): boolean {
  return isNumericColumnType(columnType) || ['date', 'datetime'].includes(columnType);
}

function boundInputType(columnType: string): 'number' | 'date' | 'text' {
  if (isNumericColumnType(columnType)) {
    return 'number';
  }
  if (columnType === 'date') {
    return 'date';
  }
  return 'text';
}

function cleanBoundInputValue(columnType: string, value: string): number | string | null {
  if (value === '') {
    return null;
  }
  if (isNumericColumnType(columnType)) {
    return Number(value);
  }
  return value;
}

function isStringBackedColumnType(columnType: string): boolean {
  return ['string', 'date', 'datetime', 'uuid', 'big_integer', 'decimal'].includes(columnType);
}

function parseUniqueConstraints(value: string): string[][] {
  return value
    .split('\n')
    .map((line) =>
      line
        .split(',')
        .map((column) => column.trim())
        .filter(Boolean)
    )
    .filter((constraint) => constraint.length > 0);
}

function RelationDetails({
  tableName,
  sourceColumn,
  target,
  targetTables,
  effectiveSchema,
  origin,
  disabled,
  onUpdateRelation,
  onRemoveRelation,
  onResetRelation
}: {
  tableName: string;
  sourceColumn: string;
  target: { target_table: string; target_column: string } | null;
  targetTables: string[];
  effectiveSchema: SchemaResponse;
  origin: 'manual' | 'suppressed' | 'inferred' | 'none';
  disabled: boolean;
  onUpdateRelation: (
    tableName: string,
    sourceColumn: string,
    targetTable: string,
    targetColumn: string
  ) => void;
  onRemoveRelation: (tableName: string, sourceColumn: string) => void;
  onResetRelation: (tableName: string, sourceColumn: string) => void;
}) {
  const [targetTable, setTargetTable] = useState(target?.target_table ?? '');
  const [targetColumn, setTargetColumn] = useState(target?.target_column ?? '');

  useEffect(() => {
    setTargetTable(target?.target_table ?? '');
    setTargetColumn(target?.target_column ?? '');
  }, [target?.target_column, target?.target_table]);

  const targetColumns = Object.keys(effectiveSchema.tables?.[targetTable]?.columns ?? {}).sort();

  return (
    <div className="schema-details-stack">
      <div className="schema-info-card">
        <p className="section-title">Relation</p>
        <p className="text-sm font-semibold text-stoneink-900">
          {tableName}.{sourceColumn}
        </p>
        <p className="overview-copy">Origin: {origin}</p>
      </div>

      <label className="schema-field">
        <span className="schema-field-label">Target table</span>
        <select
          className="overview-select"
          value={targetTable}
          onChange={(event) => {
            const nextTable = event.target.value;
            setTargetTable(nextTable);
            const [firstColumn] = Object.keys(effectiveSchema.tables?.[nextTable]?.columns ?? {}).sort();
            setTargetColumn(firstColumn ?? '');
          }}
          disabled={disabled}
        >
          <option value="">Select target table</option>
          {targetTables.map((tableOption) => (
            <option key={tableOption} value={tableOption}>
              {tableOption}
            </option>
          ))}
        </select>
      </label>

      <label className="schema-field">
        <span className="schema-field-label">Target column</span>
        <select
          className="overview-select"
          value={targetColumn}
          onChange={(event) => setTargetColumn(event.target.value)}
          disabled={disabled || !targetTable}
        >
          <option value="">Select target column</option>
          {targetColumns.map((columnName) => (
            <option key={columnName} value={columnName}>
              {columnName}
            </option>
          ))}
        </select>
      </label>

      <div className="schema-editor-actions">
        <button
          type="button"
          className="overview-secondary-button"
          onClick={() => {
            if (!targetTable || !targetColumn) {
              return;
            }
            onUpdateRelation(tableName, sourceColumn, targetTable, targetColumn);
          }}
          disabled={disabled || !targetTable || !targetColumn}
        >
          Update relation
        </button>
        <button
          type="button"
          className="overview-secondary-button"
          onClick={() => onResetRelation(tableName, sourceColumn)}
          disabled={disabled}
        >
          Reset to inferred
        </button>
        <button
          type="button"
          className="overview-secondary-button is-danger"
          onClick={() => onRemoveRelation(tableName, sourceColumn)}
          disabled={disabled}
        >
          Remove relation
        </button>
      </div>
    </div>
  );
}
