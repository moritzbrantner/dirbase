import {
  coerceIncomingRelationValue,
  coerceRelationValue,
  getColumnNames,
  getTableRows,
  shouldUseTableView,
  summarizeValue
} from './helpers';
import type { OverviewRelation, ResourceOverview } from './types';

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
  },
  mutation_capabilities: {
    create_item: true,
    update_item: true,
    delete_item: true,
    replace_object: false,
    patch_object: false
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
  },
  mutation_capabilities: {
    create_item: false,
    update_item: false,
    delete_item: false,
    replace_object: true,
    patch_object: true
  }
};

describe('resource view mode', () => {
  it('uses the table renderer only for table resources in explore mode', () => {
    expect(shouldUseTableView(tableResource, false)).toBe(true);
    expect(shouldUseTableView(tableResource, true)).toBe(false);
    expect(shouldUseTableView(objectResource, false)).toBe(false);
  });
});

describe('table helpers', () => {
  it('normalizes arrays and paginated responses into table rows', () => {
    expect(getTableRows([{ id: 1 }, 'loose value'])).toEqual([{ id: 1 }, { value: 'loose value' }]);
    expect(
      getTableRows({
        first: 1,
        prev: null,
        next: null,
        last: 1,
        page: 1,
        pages: 1,
        items: 1,
        data: [{ id: 2, title: 'hello' }]
      })
    ).toEqual([{ id: 2, title: 'hello' }]);
    expect(getTableRows({ id: 1 })).toEqual([]);
  });

  it('orders primary key columns first and merges schema, field, and sampled row names', () => {
    const resource: ResourceOverview = {
      ...tableResource,
      primary_key: 'id',
      columns: [
        { name: 'title', column_type: 'string', nullable: false, relation: null, is_primary_key: false },
        { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true }
      ],
      field_names: ['author_id', 'title']
    };

    expect(getColumnNames(resource, [{ extra: true, id: 1 }])).toEqual([
      'id',
      'title',
      'author_id',
      'extra'
    ]);
  });
});

describe('value summaries', () => {
  it('summarizes common JSON values for compact UI labels', () => {
    expect(summarizeValue(null)).toBe('null');
    expect(summarizeValue('Ada')).toBe('Ada');
    expect(summarizeValue(42)).toBe('42');
    expect(summarizeValue(false)).toBe('false');
    expect(summarizeValue([1, 2, 3])).toBe('Array(3)');
    expect(summarizeValue({ id: 1, name: 'Ada', team_id: 2, city: 'London' })).toBe(
      '{ id, name, team_id, ... }'
    );
  });
});

describe('relation value coercion', () => {
  const relation: OverviewRelation = {
    label: 'members.team_id -> teams.id',
    source_table: 'members',
    source_column: 'team_id',
    target_table: 'teams',
    target_column: 'id'
  };

  it('reads outgoing relation values from scalar and embedded rows', () => {
    expect(coerceRelationValue({ team_id: 10 }, relation)).toBe('10');
    expect(coerceRelationValue({ team_id: { id: 11, name: 'Core' } }, relation)).toBe('11');
    expect(coerceRelationValue({ team_id: { name: 'missing id' } }, relation)).toBeNull();
  });

  it('reads incoming relation values from the target column only', () => {
    expect(coerceIncomingRelationValue({ id: 'team-1' }, relation)).toBe('team-1');
    expect(coerceIncomingRelationValue({ id: { nested: true } }, relation)).toBeNull();
  });
});
