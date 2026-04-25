import { formatJson } from '../../helpers';

export function SchemaJsonDrawer({
  open,
  draftText,
  effectiveSchema,
  validationError,
  onClose,
  onChange,
  onDiscardInvalid,
  onReload
}: {
  open: boolean;
  draftText: string;
  effectiveSchema: unknown;
  validationError: string | null;
  onClose: () => void;
  onChange: (value: string) => void;
  onDiscardInvalid: () => void;
  onReload: () => void;
}) {
  return (
    <aside className={`schema-json-drawer shell-card ${open ? 'is-open' : ''}`} data-testid="schema-json-drawer">
      <div className="schema-sidebar-head">
        <div>
          <p className="section-title">Advanced JSON</p>
          <h3 className="text-lg font-semibold text-stoneink-900">Declared overlay and effective preview</h3>
        </div>
        <button type="button" className="overview-icon-button" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="schema-json-drawer-grid">
        <section className="schema-json-panel">
          <div className="schema-sidebar-head">
            <div>
              <p className="section-title">Declared</p>
              <p className="overview-copy">This is the persisted overlay sent to <code className="overview-inline-code">PUT /schema</code>.</p>
            </div>
          </div>
          <textarea
            className="schema-editor schema-json-editor"
            value={draftText}
            onChange={(event) => onChange(event.target.value)}
            spellCheck={false}
          />
          {validationError && (
            <>
              <p className="copy-status is-error">{validationError}</p>
              <div className="schema-editor-actions">
                <button type="button" className="overview-secondary-button" onClick={onDiscardInvalid}>
                  Discard invalid JSON changes
                </button>
                <button type="button" className="overview-secondary-button" onClick={onReload}>
                  Reload from server
                </button>
              </div>
            </>
          )}
        </section>

        <section className="schema-json-panel">
          <div className="schema-sidebar-head">
            <div>
              <p className="section-title">Effective preview</p>
              <p className="overview-copy">Read-only merged view of inferred schema plus the current draft.</p>
            </div>
          </div>
          <pre className="json-viewer schema-effective-preview">{formatJson(effectiveSchema)}</pre>
        </section>
      </div>
    </aside>
  );
}
