export type ResourceKind = 'table' | 'object' | 'value';

export type FilterOperator =
  | 'eq'
  | 'ne'
  | 'lt'
  | 'lte'
  | 'gt'
  | 'gte'
  | 'in'
  | 'contains'
  | 'startsWith'
  | 'endsWith'
  | 'isNull'
  | 'isNotNull';

export interface OverviewPageData {
  schema_enabled: boolean;
  data_source_kind: 'folder' | 'file';
  source_label: string;
  source_rule: string;
  resource_name_rule: string;
  stats: OverviewStats;
  resources: ResourceOverview[];
  edges: OverviewEdge[];
}

export interface OverviewStats {
  resource_count: number;
  relation_count: number;
  total_rows: number;
}

export interface ResourceOverview {
  name: string;
  kind: ResourceKind;
  row_count: number | null;
  key_count: number | null;
  primary_key: string | null;
  field_names: string[];
  row_samples: unknown[];
  columns: OverviewColumn[];
  outgoing_relations: OverviewRelation[];
  incoming_relations: OverviewRelation[];
  sample_item_id: string | null;
  query_capabilities: QueryCapabilities;
}

export interface OverviewColumn {
  name: string;
  column_type: string;
  nullable: boolean;
  relation: string | null;
  is_primary_key: boolean;
}

export interface OverviewRelation {
  label: string;
  source_table: string;
  source_column: string;
  target_table: string;
  target_column: string;
}

export interface OverviewEdge {
  source_table: string;
  source_column: string;
  target_table: string;
  target_column: string;
}

export interface QueryCapabilities {
  filter: boolean;
  sort: boolean;
  pagination: boolean;
  embed: boolean;
  item_route: boolean;
}

export interface FilterDescriptor {
  id: string;
  field: string;
  operator: FilterOperator;
  value: string;
}

export interface SortDescriptor {
  id: string;
  desc: boolean;
}

export type OverviewView = 'explore' | 'raw';

export interface OverviewUrlState {
  resource: string | null;
  view: OverviewView;
  page: number;
  perPage: number;
  filters: FilterDescriptor[];
  sorting: SortDescriptor[];
  embeds: string[];
}

export interface PaginatedResponse {
  first: number;
  prev: number | null;
  next: number | null;
  last: number;
  page: number;
  pages: number;
  items: number;
  data: unknown[];
}

export interface ResourceResponse {
  status: number;
  statusText: string;
  url: string;
  rawText: string;
  parsed: unknown;
}

export interface SchemaForeignKey {
  target_table: string;
  target_column: string;
}

export interface SchemaColumn {
  column_type?: string;
  nullable?: boolean;
  [key: string]: unknown;
}

export interface SchemaTable {
  columns?: Record<string, SchemaColumn>;
  primary_key?: string | null;
  kind?: string | null;
  foreign_keys?: Record<string, SchemaForeignKey>;
  [key: string]: unknown;
}

export interface SchemaResponse {
  tables?: Record<string, SchemaTable>;
  [key: string]: unknown;
}
