import { HighlightText } from '../shared';

type SchemaSidebarFilter = 'all' | 'changed' | 'with-relations' | 'objects' | 'relations';

export interface SchemaSidebarEntry {
  tableName: string;
  kind: string;
  relationCount: number;
  manual: boolean;
  dirty: boolean;
}

export function SchemaSidebar({
  entries,
  search,
  filter,
  selectedTableName,
  onSearchChange,
  onFilterChange,
  onSelectTable,
  mobileOpen
}: {
  entries: SchemaSidebarEntry[];
  search: string;
  filter: SchemaSidebarFilter;
  selectedTableName: string | null;
  onSearchChange: (value: string) => void;
  onFilterChange: (value: SchemaSidebarFilter) => void;
  onSelectTable: (tableName: string) => void;
  mobileOpen: boolean;
}) {
  return (
    <aside className={`schema-sidebar shell-card ${mobileOpen ? 'mobile-drawer-open' : ''}`}>
      <div className="schema-sidebar-head">
        <div>
          <p className="section-title">Tables</p>
          <h3 className="text-lg font-semibold text-stoneink-900">Browse and filter</h3>
        </div>
        <span className="overview-inline-badge">{entries.length} tables</span>
      </div>

      <label className="sidebar-search-label" htmlFor="schema-sidebar-search">
        Search tables
      </label>
      <input
        id="schema-sidebar-search"
        className="overview-input"
        value={search}
        onChange={(event) => onSearchChange(event.target.value)}
        placeholder="Search tables"
      />

      <div className="schema-filter-row">
        {[
          ['all', 'All'],
          ['changed', 'Changed'],
          ['with-relations', 'With relations'],
          ['objects', 'Objects'],
          ['relations', 'Relations']
        ].map(([value, label]) => (
          <button
            key={value}
            type="button"
            className={`schema-filter-chip ${filter === value ? 'is-active' : ''}`}
            onClick={() => onFilterChange(value as SchemaSidebarFilter)}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="schema-sidebar-list">
        {entries.map((entry) => (
          <button
            key={entry.tableName}
            type="button"
            className={`schema-sidebar-item ${selectedTableName === entry.tableName ? 'is-selected' : ''}`}
            onClick={() => onSelectTable(entry.tableName)}
          >
            <div className="resource-list-head">
              <strong>
                <HighlightText text={entry.tableName} needle={search} />
              </strong>
              <span className="overview-kind-badge">{entry.kind}</span>
            </div>
            <div className="schema-sidebar-meta">
              <span className="overview-inline-badge">{entry.relationCount} relations</span>
              <span className={`schema-origin-badge ${entry.manual ? 'is-manual' : 'is-inferred'}`}>
                {entry.manual ? 'manual' : 'inferred'}
              </span>
              {entry.dirty && <span className="schema-origin-badge is-dirty">dirty</span>}
            </div>
          </button>
        ))}
        {entries.length === 0 && <p className="overview-empty">No tables match the current filters.</p>}
      </div>
    </aside>
  );
}

export type { SchemaSidebarFilter };
