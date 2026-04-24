import { describe, expect, it } from 'vitest';

import type { OverviewUrlState, QuerySummaryChip, ResourceOverview } from '../types';
import {
  buildDrilldownFilter,
  filterResourcesBySearch,
  findSelectedResource,
  removeQuerySummaryChip
} from './overviewAppUtils';

const resources: ResourceOverview[] = [
  {
    name: 'members',
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: 'id',
    field_names: ['id', 'name', 'team_id'],
    row_samples: [],
    columns: [],
    outgoing_relations: [],
    incoming_relations: [],
    sample_item_id: '1',
    query_capabilities: {
      filter: true,
      sort: true,
      pagination: true,
      embed: true,
      item_route: true
    },
    mutation_capabilities: {
      create_item: true,
      update_item: true,
      delete_item: true,
      replace_object: false,
      patch_object: false
    }
  },
  {
    name: 'settings',
    kind: 'object',
    row_count: null,
    key_count: 2,
    primary_key: null,
    field_names: ['timezone', 'locale'],
    row_samples: [],
    columns: [],
    outgoing_relations: [],
    incoming_relations: [],
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
  }
];

const urlState: OverviewUrlState = {
  resource: 'members',
  view: 'explore',
  page: 2,
  perPage: 25,
  filters: [{ id: 'name:eq:1', field: 'name', operator: 'eq', value: 'Ada' }],
  sorting: [{ id: 'name', desc: false }],
  embeds: ['team']
};

describe('overviewAppUtils', () => {
  it('matches selected resources and falls back to the first entry', () => {
    expect(findSelectedResource(resources, 'settings')?.name).toBe('settings');
    expect(findSelectedResource(resources, 'missing')?.name).toBe('members');
  });

  it('filters resources by route name and field name', () => {
    expect(filterResourcesBySearch(resources, 'team')).toEqual([resources[0]]);
    expect(filterResourcesBySearch(resources, 'settings')).toEqual([resources[1]]);
  });

  it('removes summary chips and resets paging', () => {
    const filterChip: QuerySummaryChip = {
      id: 'name:eq:1',
      label: 'name',
      value: 'equals Ada',
      kind: 'filter',
      removeLabel: 'Remove filter on name'
    };

    expect(removeQuerySummaryChip(urlState, filterChip)).toEqual({
      ...urlState,
      page: 1,
      filters: []
    });
  });

  it('builds drilldown filters with a stable eq operator', () => {
    expect(buildDrilldownFilter('team_id', '2')).toEqual({
      id: 'team_id:eq:drilldown',
      field: 'team_id',
      operator: 'eq',
      value: '2'
    });
  });
});
