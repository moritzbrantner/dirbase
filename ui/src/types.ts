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
  server_capabilities: ServerCapabilities;
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

export interface ServerCapabilities {
  readonly: boolean;
  resource_write: boolean;
  schema_write: boolean;
  schema_infer: boolean;
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
  mutation_capabilities: MutationCapabilities;
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

export interface MutationCapabilities {
  create_item: boolean;
  update_item: boolean;
  delete_item: boolean;
  replace_object: boolean;
  patch_object: boolean;
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

export type InspectorTab = 'request' | 'selection' | 'schema';

export type LiveUpdateStatus = 'connecting' | 'live' | 'reconnecting' | 'paused';

export type MobileSurface = 'explorer' | 'resources' | 'map' | 'inspector';

export type MutationMode = 'create' | 'edit' | 'delete' | 'editObject';

export interface MutationDialogState {
  open: boolean;
  mode: MutationMode | null;
}

export interface OverviewUiState {
  selectedResource: string | null;
  selectedRow: Record<string, unknown> | null;
  inspectorTab: InspectorTab;
  liveUpdates: LiveUpdateStatus;
  mutationDialog: MutationDialogState;
  readonly: boolean;
}

export interface OverviewPreferences {
  columnVisibility: Record<string, Record<string, boolean>>;
  lastInspectorTab: InspectorTab;
  mobileSurface: MobileSurface;
}

export interface QuerySummaryChip {
  id: string;
  label: string;
  value: string;
  kind: 'filter' | 'sort' | 'embed';
  removeLabel: string;
}

export interface MutationPlan {
  method: 'POST' | 'PATCH' | 'PUT' | 'DELETE';
  path: string;
  body: string | null;
  changedKeys: string[];
  requiresConfirmation: boolean;
}
