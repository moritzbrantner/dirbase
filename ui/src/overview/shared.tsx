import { formatJson, isRecord, summarizeValue } from '../helpers';
import type { LiveUpdateStatus, ResourceOverview } from '../types';

export interface ToastMessage {
  id: number;
  tone: 'info' | 'success' | 'error';
  message: string;
}

export function SummaryCard({ label, value, copy }: { label: string; value: string | null; copy: string }) {
  return (
    <article className="summary-card">
      <span className="section-title">{label}</span>
      {value === null ? <div className="skeleton skeleton-title" /> : <strong>{value}</strong>}
      <p>{copy}</p>
    </article>
  );
}

export function ToastViewport({ toasts }: { toasts: ToastMessage[] }) {
  return (
    <div className="toast-viewport" aria-live="polite" aria-atomic="true">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast-message is-${toast.tone}`}>
          {toast.message}
        </div>
      ))}
    </div>
  );
}

export function HighlightText({ text, needle }: { text: string; needle: string }) {
  if (!needle.trim()) {
    return <>{text}</>;
  }

  const lowerText = text.toLowerCase();
  const lowerNeedle = needle.trim().toLowerCase();
  const index = lowerText.indexOf(lowerNeedle);
  if (index === -1) {
    return <>{text}</>;
  }

  const start = text.slice(0, index);
  const match = text.slice(index, index + lowerNeedle.length);
  const end = text.slice(index + lowerNeedle.length);
  return (
    <>
      {start}
      <mark>{match}</mark>
      {end}
    </>
  );
}

export function TableSkeleton() {
  return (
    <div className="table-skeleton">
      {Array.from({ length: 6 }).map((_, rowIndex) => (
        <div key={rowIndex} className="table-skeleton-row">
          {Array.from({ length: 5 }).map((__, cellIndex) => (
            <div key={cellIndex} className="skeleton skeleton-cell" />
          ))}
        </div>
      ))}
    </div>
  );
}

export function renderCellValue(value: unknown) {
  if (value === null || value === undefined) {
    return <span className="cell-muted">null</span>;
  }
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return <span>{String(value)}</span>;
  }
  if (Array.isArray(value)) {
    return (
      <details className="cell-disclosure">
        <summary>{`Array(${value.length})`}</summary>
        <pre className="json-viewer compact">{formatJson(value)}</pre>
      </details>
    );
  }
  if (isRecord(value)) {
    return (
      <details className="cell-disclosure">
        <summary>{summarizeValue(value)}</summary>
        <pre className="json-viewer compact">{formatJson(value)}</pre>
      </details>
    );
  }
  return <span>{String(value)}</span>;
}

export function groupResources(resources: ResourceOverview[]) {
  return resources.reduce<Record<ResourceOverview['kind'], ResourceOverview[]>>(
    (groups, resource) => {
      groups[resource.kind].push(resource);
      return groups;
    },
    { table: [], object: [], value: [] }
  );
}

export function renderCapabilityChip(label: string, enabled: boolean) {
  return (
    <span className={`capability-chip ${enabled ? 'is-enabled' : ''}`} key={label}>
      {label}
    </span>
  );
}

export function renderLiveUpdateLabel(status: LiveUpdateStatus) {
  switch (status) {
    case 'live':
      return 'Live';
    case 'reconnecting':
      return 'Reconnecting';
    case 'paused':
      return 'Paused';
    default:
      return 'Connecting';
  }
}

export function getJsonValidationError(value: string) {
  try {
    JSON.parse(value);
    return null;
  } catch (caught) {
    return caught instanceof Error ? caught.message : 'Invalid JSON.';
  }
}
