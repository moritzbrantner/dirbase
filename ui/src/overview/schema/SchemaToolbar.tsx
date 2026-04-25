export function SchemaToolbar({
  dirty,
  stale,
  busy,
  validationError,
  status,
  savePath,
  readonly,
  onReload,
  onInfer,
  onSave,
  onToggleJson
}: {
  dirty: boolean;
  stale: boolean;
  busy: boolean;
  validationError: string | null;
  status: string | null;
  savePath: string;
  readonly: boolean;
  onReload: () => void;
  onInfer: () => void;
  onSave: () => void;
  onToggleJson: () => void;
}) {
  return (
    <section className="schema-toolbar shell-card">
      <div className="schema-toolbar-head">
        <div className="grid gap-1">
          <div className="schema-toolbar-title-row">
            <p className="section-title">Schema workspace</p>
            <span className={`status-pill ${dirty ? 'is-warn' : ''}`}>{dirty ? 'Draft changed' : 'In sync'}</span>
            {stale && <span className="status-pill is-error">Server changed</span>}
            {readonly && <span className="status-pill is-warn">Read-only</span>}
          </div>
          <h2 className="text-xl font-semibold tracking-tight text-stoneink-900">Graph-first schema editing</h2>
          <p className="overview-copy">
            Edits stay local until save. Persisted overlay path <code className="overview-inline-code">{savePath}</code>
          </p>
        </div>
        <div className="schema-toolbar-actions">
          <button type="button" className="overview-secondary-button" onClick={onReload}>
            Reload
          </button>
          <button
            type="button"
            className="overview-secondary-button"
            onClick={onInfer}
            disabled={readonly || busy}
          >
            {busy ? 'Working…' : 'Infer from data'}
          </button>
          <button type="button" className="overview-secondary-button" onClick={onToggleJson}>
            JSON
          </button>
          <button
            type="button"
            className="overview-secondary-button is-primary"
            onClick={onSave}
            disabled={readonly || busy || Boolean(validationError)}
          >
            {busy ? 'Working…' : 'Save'}
          </button>
        </div>
      </div>

      <div className="schema-toolbar-status-row">
        <span className={`schema-validation-pill ${validationError ? 'is-error' : 'is-ready'}`}>
          {validationError ? validationError : 'Schema draft validates locally.'}
        </span>
        {status && <span className="overview-inline-badge">{status}</span>}
      </div>
    </section>
  );
}
