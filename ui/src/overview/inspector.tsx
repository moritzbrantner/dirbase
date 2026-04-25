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
  selectedRow,
  selectedTab,
  outgoingRelations,
  incomingRelations,
  readonly,
  mobileOpen,
  requestPath,
  requestUrl,
  onTabChange,
  onCopy,
  onOpenRequest,
  onDrilldownOutgoing,
  onDrilldownIncoming
}: {
  resource: ResourceOverview | null;
  response: ResourceResponse | undefined;
  selectedRow: Record<string, unknown> | null;
  selectedTab: InspectorTab;
  outgoingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  incomingRelations: Array<{ relation: OverviewRelation; value: string | null }>;
  readonly: boolean;
  mobileOpen: boolean;
  requestPath: string;
  requestUrl: string;
  onTabChange: (tab: InspectorTab) => void;
  onCopy: (value: string) => Promise<void>;
  onOpenRequest: () => void;
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
          <h2 className="text-xl font-semibold tracking-tight text-stoneink-900">Request and detail</h2>
        </div>
        {readonly && <span className="status-pill is-warn">Read-only</span>}
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
      </div>

      {selectedTab === 'request' && (
        <section className="inspector-section">
          <code className="request-path">
            GET {requestPath}
          </code>
          <div className="request-action-grid">
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(requestPath)}>
              Copy URL
            </button>
            <button type="button" className="overview-secondary-button" onClick={() => void onCopy(curlExample)}>
              Copy curl
            </button>
            <button type="button" className="overview-secondary-button" onClick={onOpenRequest}>
              Open request
            </button>
          </div>
          {response && (
            <div className="request-meta-stack">
              <div className="overview-inline-list">
                <span className={`status-pill ${response.status >= 400 ? 'is-error' : ''}`}>
                  {response.status} {response.statusText}
                </span>
                <span className="overview-inline-badge">{response.url}</span>
              </div>
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
            <section className="grid gap-3">
              <div>
                <h3 className="text-sm font-semibold text-stoneink-900">Snapshot</h3>
                <p className="overview-copy">
                  {resource
                    ? truncate(summarizeValue(response?.parsed ?? resource.row_samples[0] ?? null), 180)
                    : 'Choose a resource to inspect it here.'}
                </p>
              </div>
              {resource && resource.row_samples.length > 0 && (
                <pre className="json-viewer">{formatJson(resource.row_samples[0])}</pre>
              )}
            </section>
          )}
        </section>
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
            <h2 className="text-2xl font-semibold tracking-tight text-stoneink-900">
              {renderMutationTitle(mode, resource.kind)}
            </h2>
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
                <span>Use `PUT` to replace the full document instead of sending only changed keys.</span>
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
      <section className="grid gap-3">
        <div>
          <h3 className="text-sm font-semibold text-stoneink-900">Selected row</h3>
          <p className="overview-copy">
            {itemRoute ? (
              <>
                Item route <code className="overview-inline-code">{itemRoute}</code>
              </>
            ) : (
              'Select a row to inspect the raw JSON payload.'
            )}
          </p>
        </div>
      </section>

      <section className="grid gap-2">
        <h3 className="text-sm font-semibold text-stoneink-900">Relations out</h3>
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
            <p className="overview-empty">No outgoing drill-down is available for this row.</p>
          )}
        </div>
      </section>

      <section className="grid gap-2">
        <h3 className="text-sm font-semibold text-stoneink-900">Relations in</h3>
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
            <p className="overview-empty">No incoming drill-down is available for this row.</p>
          )}
        </div>
      </section>

      <section className="grid gap-2">
        <h3 className="text-sm font-semibold text-stoneink-900">JSON</h3>
        <pre className="json-viewer">{formatJson(row)}</pre>
      </section>
    </div>
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
