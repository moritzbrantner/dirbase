import { shouldUseTableView } from './helpers';
import type { ResourceOverview } from './types';

const tableResource: ResourceOverview = {
  name: 'posts',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'title'],
  row_samples: [],
  columns: [],
  outgoing_relations: [],
  incoming_relations: [],
  sample_item_id: '1',
  query_capabilities: {
    filter: true,
    sort: true,
    pagination: true,
    embed: false,
    item_route: true
  }
};

const objectResource: ResourceOverview = {
  ...tableResource,
  name: 'settings',
  kind: 'object',
  row_count: null,
  key_count: 2,
  primary_key: null,
  sample_item_id: null,
  query_capabilities: {
    filter: false,
    sort: false,
    pagination: false,
    embed: false,
    item_route: false
  }
};

describe('resource view mode', () => {
  it('uses the table renderer only for table resources in explore mode', () => {
    expect(shouldUseTableView(tableResource, false)).toBe(true);
    expect(shouldUseTableView(tableResource, true)).toBe(false);
    expect(shouldUseTableView(objectResource, false)).toBe(false);
  });
});
