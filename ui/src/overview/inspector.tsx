import { useEffect, useState } from 'react';

import { formatJson, isPaginatedResponse, summarizeValue, truncate } from '../helpers';
import { buildMutationPlan } from '../overviewUtils';
import type {
  InspectorTab,
  MutationMode,
  MutationPlan,
  OverviewRelation,
  ResourceOverview,
  ResourceResponse
} from '../types';
import { getJsonValidationError } from './shared';

export function InspectorPanel({
  resource,
  response,
  schemaDraft,
  schemaStatus,
  schemaDiffSummary,
  schemaValidationError,
  selectedRow,
  selectedTab,
  outgoingRelations,
  incomingRelations,
  readonly,
  mobileOpen,
  schemaBusy,
  canSaveSchema,
  canInferSchema,
  requestPath,
  requestUrl,
  onTabChange,
  onSchemaDraftChange,
  onCopy,
  onOpenRequest,
  onReloadSchema,
  onSaveSchema,
  onInferSchema,
  onDrilldownOutgoing,
  onDrilldownIncoming
}: {
  resource: ResourceOverview | null;
  response: ResourceResponse | undefined;
  schemaDraft: string;
  schemaStatus: string | null;
  schemaDiffSummary: string[];
  schemaValidationError: string | null;
  selectedRow: Record<string, unknown> | null;
  selectedTab: InspectorTab;
  outgoingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  incomingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  readonly: boolean;
  mobileOpen: boolean;
  schemaBusy: boolean;
  canSaveSchema: boolean;
  canInferSchema: boolean;
  requestPath: string;
  requestUrl: string;
  onTabChange: (tab: InspectorTab) => void;
  onSchemaDraftChange: (value: string) => void;
  onCopy: (value: string) => Promise<void>;
  onOpenRequest: () => void;
  onReloadSchema: () => void;
  onSaveSchema: () => void;
  onInferSchema: () => void;
  onDrilldownOutgoing: (entry: { relation: OverviewRelation; value: string | null }) => void;
  onDrilldownIncoming: (entry: { relation: OverviewRelation; value: string | null }) => void;
}) {
  const curlExample = `curl -H 'Accept: application/json' '${requestUrl}'`;

  return (
    <aside
      className={`workspace-details shell-card ${mobileOpen ? 'mobile-sheet-open' : ''}`}
      data-testid="inspector-panel"
    >
      <div className="overview-panel-head">
        <div>
          <p className="section-title">Inspector</p>
          <h2>Request, selection, schema</h2>
        </div>
      </div>

      <div className="inspector-tab-row" role="tablist" aria-label="Inspector tabs">
        <button type="button" className={selectedTab === 'request' ? 'is-active' : ''} onClick={() => onTabChange('request')}>
          Request
        </button>
        <button
          type="button"
          className={selectedTab === 'selection' ? 'is-active' : ''}
          onClick={() => onTabChange('selection')}
        >
          Selection
        </button>
        <button type="button" className={selectedTab === 'schema' ? 'is-active' : ''} onClick={() => onTabChange('schema')}>
          Schema
        </button>
      </div>

      {selectedTab === 'request' && (
        <section className="inspector-section">
          <div className="request-header-row">
            <span className="overview-inline-code">
              <span className="overview-method">GET</span>
              {requestPath}
            </span>
          </div>
          <div className="request-action-grid">
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(requestPath)}>
              Copy relative URL
            </button>
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(curlExample)}>
              Copy curl
            </button>
            <button type="button" className="overview-secondary-button" onClick={onOpenRequest}>
              Open request
            </button>
          </div>
          <code className="request-path">{requestPath}</code>
          {response && (
            <div className="request-meta-stack">
              <span className={`status-pill ${response.status >= 400 ? 'is-error' : ''}`}>
                {response.status} {response.statusText}
              </span>
              {isPaginatedResponse(response.parsed) && (
                <div className="request-page-metadata">
                  <span className="overview-inline-badge">Page {response.parsed.page}</span>
                  <span className="overview-inline-badge">{response.parsed.items} items</span>
                  <span className="overview-inline-badge">{response.parsed.pages} pages</span>
                </div>
              )}
            </div>
          )}
        </section>
      )}

      {selectedTab === 'selection' && (
        <section className="inspector-section">
          {resource?.kind === 'table' && selectedRow ? (
            <SelectionPanel
              resource={resource}
              row={selectedRow}
              outgoingRelations={outgoingRelations}
              incomingRelations={incomingRelations}
              onDrilldownOutgoing={onDrilldownOutgoing}
              onDrilldownIncoming={onDrilldownIncoming}
            />
          ) : (
            <section className="resource-snapshot">
              <h3>Resource snapshot</h3>
              <p className="overview-copy">
                {resource
                  ? truncate(summarizeValue(response?.parsed ?? resource.row_samples[0] ?? null), 180)
                  : 'Choose a resource to inspect it here.'}
              </p>
              {resource && resource.row_samples.length > 0 && (
                <pre className="json-viewer">{formatJson(resource.row_samples[0])}</pre>
              )}
            </section>
          )}
        </section>
      )}

      {selectedTab === 'schema' && (
        <SchemaEditorPanel
          draft={schemaDraft}
          status={schemaStatus}
          diffSummary={schemaDiffSummary}
          validationError={schemaValidationError}
          readonly={readonly}
          busy={schemaBusy}
          canSave={canSaveSchema}
          canInfer={canInferSchema}
          onChange={onSchemaDraftChange}
          onReload={onReloadSchema}
          onSave={onSaveSchema}
          onInfer={onInferSchema}
        />
      )}
    </aside>
  );
}

export function MutationDialog({
  open,
  mode,
  resource,
  selectedRow,
  objectValue,
  onClose,
  onSubmit
}: {
  open: boolean;
  mode: MutationMode | null;
  resource: ResourceOverview | null;
  selectedRow: Record<string, unknown> | null;
  objectValue: unknown;
  onClose: () => void;
  onSubmit: (plan: MutationPlan) => Promise<void>;
}) {
  const [draftText, setDraftText] = useState('{}');
  const [replaceFullItem, setReplaceFullItem] = useState(false);
  const [confirmAction, setConfirmAction] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (!open || !resource || !mode) {
      return;
    }
    const sourceValue =
      mode === 'create'
        ? {}
        : mode === 'editObject'
          ? objectValue ?? {}
          : selectedRow ?? {};
    setDraftText(formatJson(sourceValue));
    setReplaceFullItem(false);
    setConfirmAction(false);
    setError(null);
  }, [mode, objectValue, open, resource, selectedRow]);

  if (!open || !resource || !mode) {
    return null;
  }

  const sourceValue =
    mode === 'create' ? {} : mode === 'editObject' ? objectValue ?? {} : selectedRow ?? {};
  const validationError = mode === 'delete' ? null : getJsonValidationError(draftText);

  let plan: MutationPlan | null = null;
  let planError: string | null = null;
  try {
    plan = buildMutationPlan({
      resource,
      mode,
      originalValue: sourceValue,
      draftText,
      replaceFullItem
    });
  } catch (caught) {
    planError = caught instanceof Error ? caught.message : 'Unable to build mutation request.';
  }

  const submitDisabled =
    pending ||
    Boolean(validationError) ||
    Boolean(planError) ||
    !plan ||
    (plan?.method === 'PATCH' && plan.changedKeys.length === 0) ||
    ((plan?.requiresConfirmation ?? false) && !confirmAction);

  return (
    <div className="dialog-backdrop" role="presentation" onClick={onClose}>
      <div
        className="mutation-dialog"
        role="dialog"
        aria-modal="true"
        data-testid="mutation-dialog"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="overview-panel-head">
          <div>
            <p className="section-title">Mutation</p>
            <h2>{renderMutationTitle(mode, resource.kind)}</h2>
          </div>
          <button type="button" className="overview-icon-button" onClick={onClose}>
            Close
          </button>
        </div>

        {mode !== 'delete' && (
          <>
            <textarea
              className="schema-editor mutation-editor"
              value={draftText}
              onChange={(event) => setDraftText(event.target.value)}
              spellCheck={false}
            />
            {(mode === 'edit' || mode === 'editObject') && (
              <label className="mutation-toggle">
                <input
                  type="checkbox"
                  checked={replaceFullItem}
                  onChange={(event) => {
                    setReplaceFullItem(event.target.checked);
                    setConfirmAction(false);
                  }}
                />
                <span>Replace the full document with `PUT` instead of sending only changed keys with `PATCH`.</span>
              </label>
            )}
          </>
        )}

        {plan && (
          <div className="mutation-plan-card">
            <p className="section-title">Request</p>
            <code className="request-path">
              {plan.method} {plan.path}
            </code>
            {plan.changedKeys.length > 0 && <p className="overview-copy">Changed keys: {plan.changedKeys.join(', ')}</p>}
            {plan.method === 'PATCH' && plan.changedKeys.length === 0 && (
              <p className="copy-status is-error">No changed keys detected. Edit the JSON or switch to full replace.</p>
            )}
          </div>
        )}

        {(plan?.requiresConfirmation ?? false) && (
          <label className="mutation-toggle">
            <input
              type="checkbox"
              checked={confirmAction}
              onChange={(event) => setConfirmAction(event.target.checked)}
            />
            <span>
              {plan?.method === 'DELETE'
                ? 'I understand this delete cannot be undone.'
                : 'I understand this will replace the full document with PUT.'}
            </span>
          </label>
        )}

        {(validationError || planError || error) && (
          <p className="copy-status is-error">{validationError ?? planError ?? error}</p>
        )}

        <div className="mutation-dialog-actions">
          <button type="button" className="overview-secondary-button" onClick={onClose} disabled={pending}>
            Cancel
          </button>
          <button
            type="button"
            className="overview-secondary-button"
            onClick={async () => {
              if (!plan || submitDisabled) {
                return;
              }
              setPending(true);
              setError(null);
              try {
                await onSubmit(plan);
                onClose();
              } catch (caught) {
                setError(caught instanceof Error ? caught.message : 'Request failed.');
              } finally {
                setPending(false);
              }
            }}
            disabled={submitDisabled}
          >
            {pending ? 'Submitting…' : `${plan?.method ?? 'Submit'} request`}
          </button>
        </div>
      </div>
    </div>
  );
}

function SelectionPanel({
  resource,
  row,
  outgoingRelations,
  incomingRelations,
  onDrilldownOutgoing,
  onDrilldownIncoming
}: {
  resource: ResourceOverview;
  row: Record<string, unknown>;
  outgoingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  incomingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  onDrilldownOutgoing: (entry: { relation: OverviewRelation; value: string | null }) => void;
  onDrilldownIncoming: (entry: { relation: OverviewRelation; value: string | null }) => void;
}) {
  const itemRoute =
    resource.primary_key && row[resource.primary_key] !== undefined
      ? `/${resource.name}/${String(row[resource.primary_key])}`
      : null;

  return (
    <div className="details-stack">
      <section>
        <h3>Selected row</h3>
        <p className="overview-copy">
          {itemRoute ? (
            <>
              Item route: <code className="overview-inline-code">{itemRoute}</code>
            </>
          ) : (
            'Select a row to inspect the raw JSON payload.'
          )}
        </p>
        <pre className="json-viewer">{formatJson(row)}</pre>
      </section>

      <section>
        <h3>Outgoing relations</h3>
        <div className="relation-link-list">
          {outgoingRelations.length > 0 ? (
            outgoingRelations.map((entry) => (
              <button
                key={`${entry.relation.source_table}:${entry.relation.source_column}`}
                type="button"
                className="relation-link-button"
                onClick={() => onDrilldownOutgoing(entry)}
              >
                <strong>{entry.relation.target_table}</strong>
                <span>
                  {entry.relation.target_column} = {entry.value}
                </span>
              </button>
            ))
          ) : (
            <p className="overview-empty">No outgoing relation drill-down is available for this row.</p>
          )}
        </div>
      </section>

      <section>
        <h3>Incoming relations</h3>
        <div className="relation-link-list">
          {incomingRelations.length > 0 ? (
            incomingRelations.map((entry) => (
              <button
                key={`${entry.relation.source_table}:${entry.relation.source_column}:incoming`}
                type="button"
                className="relation-link-button"
                onClick={() => onDrilldownIncoming(entry)}
              >
                <strong>{entry.relation.source_table}</strong>
                <span>
                  {entry.relation.source_column} = {entry.value}
                </span>
              </button>
            ))
          ) : (
            <p className="overview-empty">No incoming relation drill-down is available for this row.</p>
          )}
        </div>
      </section>
    </div>
  );
}

function SchemaEditorPanel({
  draft,
  status,
  diffSummary,
  validationError,
  readonly,
  busy,
  canSave,
  canInfer,
  onChange,
  onReload,
  onSave,
  onInfer
}: {
  draft: string;
  status: string | null;
  diffSummary: string[];
  validationError: string | null;
  readonly: boolean;
  busy: boolean;
  canSave: boolean;
  canInfer: boolean;
  onChange: (value: string) => void;
  onReload: () => void;
  onSave: () => void;
  onInfer: () => void;
}) {
  return (
    <section className="inspector-section">
      <div className="schema-panel-head">
        <div>
          <h3>Schema editor</h3>
          <p className="overview-copy">
            Edit the JSON schema overlay directly. Save uses `PUT /schema`; infer uses `POST /schema/infer`.
          </p>
        </div>
        {readonly && <span className="status-pill is-warn">Read-only mode</span>}
      </div>

      <textarea
        className="schema-editor"
        value={draft}
        onChange={(event) => onChange(event.target.value)}
        readOnly={readonly}
        spellCheck={false}
        data-testid="schema-editor"
      />

      {validationError && <p className="copy-status is-error">{validationError}</p>}
      {diffSummary.length > 0 && (
        <div className="schema-diff-summary">
          <p className="section-title">Diff summary</p>
          {diffSummary.map((line) => (
            <p key={line} className="overview-copy">
              {line}
            </p>
          ))}
        </div>
      )}

      <div className="schema-editor-actions">
        <button type="button" className="overview-secondary-button" onClick={onReload}>
          Reload
        </button>
        <button
          type="button"
          className="overview-secondary-button"
          onClick={onInfer}
          disabled={readonly || !canInfer || busy}
        >
          {busy ? 'Working…' : 'Infer from data'}
        </button>
        <button
          type="button"
          className="overview-secondary-button"
          onClick={onSave}
          disabled={readonly || !canSave || busy}
        >
          {busy ? 'Working…' : 'Save'}
        </button>
      </div>

      {status && <p className="copy-status">{status}</p>}
    </section>
  );
}

function renderMutationTitle(mode: MutationMode, kind: ResourceOverview['kind']) {
  if (mode === 'create') {
    return 'Create row';
  }
  if (mode === 'delete') {
    return 'Delete row';
  }
  if (kind === 'object') {
    return 'Edit object';
  }
  return 'Edit row';
}
