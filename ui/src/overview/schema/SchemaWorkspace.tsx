import { useEffect, useMemo, useState } from 'react';
import type { Connection } from '@xyflow/react';

import {
  getSchemaTableNames,
  getTableRelationCount,
  isSchemaTableManual,
  removeDeclaredRelationship,
  resetDeclaredRelationship,
  setDeclaredColumnOverride,
  setDeclaredPrimaryKey,
  setDeclaredTableKind,
  stageRelationshipFromConnection,
  upsertDeclaredRelationship
} from '../../schemaWorkspace';
import type {
  DeclaredSchemaResponse,
  ResourceOverview,
  SchemaMobileSurface,
  SchemaResponse,
  SchemaWorkspaceSelection
} from '../../types';
import { SchemaCanvas } from './SchemaCanvas';
import { SchemaDetailsPanel } from './SchemaDetailsPanel';
import { SchemaJsonDrawer } from './SchemaJsonDrawer';
import { SchemaSidebar, type SchemaSidebarFilter } from './SchemaSidebar';
import { SchemaToolbar } from './SchemaToolbar';

export function SchemaWorkspace({
  resources,
  inferredSchema,
  effectiveSchema,
  declaredDraft,
  declaredDraftText,
  schemaDirty,
  schemaStale,
  schemaBusy,
  schemaStatus,
  jsonDraftError,
  schemaValidationError,
  jsonDrawerOpen,
  savePath,
  readonly,
  selectedResourceName,
  schemaMobileSurface,
  onSelectResource,
  onSchemaMobileSurfaceChange,
  onToggleJsonDrawer,
  onDeclaredDraftChange,
  onDeclaredDraftTextChange,
  onReload,
  onInfer,
  onSave,
  onDiscardInvalidJson
}: {
  resources: ResourceOverview[];
  inferredSchema: SchemaResponse;
  effectiveSchema: SchemaResponse;
  declaredDraft: DeclaredSchemaResponse;
  declaredDraftText: string;
  schemaDirty: boolean;
  schemaStale: boolean;
  schemaBusy: boolean;
  schemaStatus: string | null;
  jsonDraftError: string | null;
  schemaValidationError: string | null;
  jsonDrawerOpen: boolean;
  savePath: string;
  readonly: boolean;
  selectedResourceName: string | null;
  schemaMobileSurface: SchemaMobileSurface;
  onSelectResource: (resourceName: string) => void;
  onSchemaMobileSurfaceChange: (surface: SchemaMobileSurface) => void;
  onToggleJsonDrawer: () => void;
  onDeclaredDraftChange: (nextDraft: DeclaredSchemaResponse, status?: string | null) => void;
  onDeclaredDraftTextChange: (value: string) => void;
  onReload: () => void;
  onInfer: () => void;
  onSave: () => void;
  onDiscardInvalidJson: () => void;
}) {
  const [selection, setSelection] = useState<SchemaWorkspaceSelection | null>(null);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<SchemaSidebarFilter>('all');
  const structuredEditingDisabled = Boolean(jsonDraftError);

  const discoveredTableNames = useMemo(() => resources.map((resource) => resource.name), [resources]);
  const tableNames = useMemo(
    () => getSchemaTableNames(inferredSchema, declaredDraft, discoveredTableNames),
    [declaredDraft, discoveredTableNames, inferredSchema]
  );

  useEffect(() => {
    if (selectedResourceName && tableNames.includes(selectedResourceName)) {
      setSelection((current) =>
        current?.tableName === selectedResourceName ? current : { kind: 'table', tableName: selectedResourceName }
      );
      return;
    }

    if (!selection && tableNames.length > 0) {
      const [firstTable] = tableNames;
      setSelection({ kind: 'table', tableName: firstTable });
      onSelectResource(firstTable);
    }
  }, [onSelectResource, selectedResourceName, selection, tableNames]);

  const sidebarEntries = useMemo(() => {
    return tableNames
      .map((tableName) => ({
        tableName,
        kind: effectiveSchema.tables?.[tableName]?.kind ?? 'unknown',
        relationCount: getTableRelationCount(effectiveSchema, tableName),
        manual: isSchemaTableManual(declaredDraft, tableName),
        dirty: schemaDirty && isSchemaTableManual(declaredDraft, tableName)
      }))
      .filter((entry) => {
        const matchesSearch = !search.trim() || entry.tableName.toLowerCase().includes(search.trim().toLowerCase());
        if (!matchesSearch) {
          return false;
        }
        switch (filter) {
          case 'changed':
            return entry.manual;
          case 'with-relations':
            return entry.relationCount > 0;
          case 'objects':
            return entry.kind === 'object';
          case 'relations':
            return entry.kind === 'relation';
          default:
            return true;
        }
      });
  }, [declaredDraft, effectiveSchema, filter, schemaDirty, search, tableNames]);

  function selectTable(tableName: string) {
    setSelection({ kind: 'table', tableName });
    onSelectResource(tableName);
  }

  function selectColumn(tableName: string, columnName: string) {
    setSelection({ kind: 'column', tableName, columnName });
    onSelectResource(tableName);
  }

  function selectRelation(tableName: string, sourceColumn: string) {
    setSelection({ kind: 'relation', tableName, relationSourceColumn: sourceColumn });
    onSelectResource(tableName);
  }

  function handleCreateRelationship(connection: Connection) {
    const staged = stageRelationshipFromConnection(declaredDraft, connection);
    if (!staged) {
      return;
    }
    onDeclaredDraftChange(
      staged.declared,
      `Staged ${staged.selection.tableName}.${staged.selection.relationSourceColumn}. Save schema to persist it.`
    );
    setSelection({
      kind: 'relation',
      tableName: staged.selection.tableName,
      relationSourceColumn: staged.selection.relationSourceColumn
    });
    onSelectResource(staged.selection.tableName);
  }

  return (
    <div className="schema-workspace-shell">
      <SchemaToolbar
        dirty={schemaDirty}
        stale={schemaStale}
        busy={schemaBusy}
        validationError={schemaValidationError}
        status={schemaStatus}
        savePath={savePath}
        readonly={readonly}
        onReload={onReload}
        onInfer={onInfer}
        onSave={onSave}
        onToggleJson={onToggleJsonDrawer}
      />

      <div className="schema-mobile-surface-bar">
        {[
          ['tables', 'Tables'],
          ['graph', 'Graph'],
          ['details', 'Details'],
          ['json', 'JSON']
        ].map(([surface, label]) => (
          <button
            key={surface}
            type="button"
            className={schemaMobileSurface === surface ? 'is-active' : ''}
            onClick={() => onSchemaMobileSurfaceChange(surface as SchemaMobileSurface)}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="schema-workspace-grid">
        <SchemaSidebar
          entries={sidebarEntries}
          search={search}
          filter={filter}
          selectedTableName={selection?.tableName ?? null}
          onSearchChange={setSearch}
          onFilterChange={setFilter}
          onSelectTable={selectTable}
          mobileOpen={schemaMobileSurface === 'tables'}
        />

        <section className={`schema-canvas-panel shell-card ${schemaMobileSurface === 'graph' ? 'mobile-drawer-open' : ''}`}>
          <div className="schema-sidebar-head">
            <div>
              <p className="section-title">Graph</p>
              <h3 className="text-lg font-semibold text-stoneink-900">Visual relations and columns</h3>
            </div>
          </div>
          <p className="overview-copy">
            Select tables and columns directly on the graph. Drag from a source column to a target column to stage a relation.
          </p>
          <SchemaCanvas
            resources={resources}
            effectiveTables={effectiveSchema.tables ?? {}}
            selection={selection}
            readonly={readonly}
            structuredEditingDisabled={structuredEditingDisabled}
            onSelectTable={selectTable}
            onSelectColumn={selectColumn}
            onSelectRelation={selectRelation}
            onCreateRelationship={handleCreateRelationship}
          />
        </section>

        <SchemaDetailsPanel
          selection={selection}
          discoveredTables={discoveredTableNames}
          effectiveSchema={effectiveSchema}
          inferredSchema={inferredSchema}
          declaredDraft={declaredDraft}
          readonly={readonly}
          mobileOpen={schemaMobileSurface === 'details'}
          structuredEditingDisabled={structuredEditingDisabled}
          onSelectRelation={selectRelation}
          onSetTableKind={(tableName, kind) =>
            onDeclaredDraftChange(setDeclaredTableKind(declaredDraft, tableName, kind))
          }
          onSetPrimaryKey={(tableName, primaryKey) =>
            onDeclaredDraftChange(setDeclaredPrimaryKey(declaredDraft, tableName, primaryKey))
          }
          onSetColumnOverride={(tableName, columnName, next) =>
            onDeclaredDraftChange(
              setDeclaredColumnOverride(declaredDraft, effectiveSchema, tableName, columnName, next)
            )
          }
          onUpdateRelation={(tableName, sourceColumn, targetTable, targetColumn) => {
            onDeclaredDraftChange(
              upsertDeclaredRelationship(declaredDraft, {
                sourceTable: tableName,
                sourceColumn,
                targetTable,
                targetColumn
              })
            );
            selectRelation(tableName, sourceColumn);
          }}
          onRemoveRelation={(tableName, sourceColumn) =>
            onDeclaredDraftChange(
              removeDeclaredRelationship(declaredDraft, inferredSchema, tableName, sourceColumn)
            )
          }
          onResetRelation={(tableName, sourceColumn) =>
            onDeclaredDraftChange(resetDeclaredRelationship(declaredDraft, tableName, sourceColumn))
          }
        />
      </div>

      <SchemaJsonDrawer
        open={jsonDrawerOpen || schemaMobileSurface === 'json'}
        draftText={declaredDraftText}
        effectiveSchema={effectiveSchema}
        validationError={jsonDraftError}
        onClose={onToggleJsonDrawer}
        onChange={onDeclaredDraftTextChange}
        onDiscardInvalid={onDiscardInvalidJson}
        onReload={onReload}
      />
    </div>
  );
}
