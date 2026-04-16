import {
  buildMutationPlan,
  buildQuerySummaryChips,
  getVisibleMutationActions,
  loadOverviewPreferences
} from './overviewUtils';
import type { ResourceOverview, ServerCapabilities } from './types';

const writableServer: ServerCapabilities = {
  readonly: false,
  resource_write: true,
  schema_write: true,
  schema_infer: true
};

const tableResource: ResourceOverview = {
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

describe('buildQuerySummaryChips', () => {
  it('renders filter, sort, and embed chips from URL state', () => {
    expect(
      buildQuerySummaryChips({
        filters: [{ id: 'status:eq:1', field: 'status', operator: 'eq', value: 'active' }],
        sorting: [{ id: 'name', desc: true }],
        embeds: ['team_id']
      })
    ).toEqual([
      {
        id: 'status:eq:1',
        kind: 'filter',
        label: 'status',
        value: 'equals active',
        removeLabel: 'Remove filter on status'
      },
      {
        id: 'sort:name',
        kind: 'sort',
        label: 'name',
        value: 'descending',
        removeLabel: 'Remove sort on name'
      },
      {
        id: 'embed:team_id',
        kind: 'embed',
        label: 'team_id',
        value: 'embedded',
        removeLabel: 'Remove embed on team_id'
      }
    ]);
  });
});

describe('buildMutationPlan', () => {
  it('uses PATCH with only changed keys for row edits by default', () => {
    const plan = buildMutationPlan({
      resource: tableResource,
      mode: 'edit',
      originalValue: { id: 1, name: 'Ada', team_id: 2 },
      draftText: JSON.stringify({ id: 1, name: 'Grace', team_id: 2 }),
      replaceFullItem: false
    });

    expect(plan.method).toBe('PATCH');
    expect(plan.path).toBe('/members/1');
    expect(plan.body).toBe('{"name":"Grace"}');
    expect(plan.changedKeys).toEqual(['name']);
  });

  it('switches to PUT for full object replacement after explicit selection', () => {
    const plan = buildMutationPlan({
      resource: objectResource,
      mode: 'editObject',
      originalValue: { theme: 'warm', locale: 'en' },
      draftText: JSON.stringify({ theme: 'cool', locale: 'de' }),
      replaceFullItem: true
    });

    expect(plan.method).toBe('PUT');
    expect(plan.path).toBe('/settings');
    expect(plan.changedKeys).toEqual(['theme', 'locale']);
    expect(plan.requiresConfirmation).toBe(true);
  });
});

describe('getVisibleMutationActions', () => {
  it('hides write actions when the server is not writable', () => {
    expect(
      getVisibleMutationActions(tableResource, { ...writableServer, resource_write: false }, { id: 1 })
    ).toEqual({
      createRow: false,
      editRow: false,
      deleteRow: false,
      editObject: false
    });
  });

  it('returns object editing availability from capabilities', () => {
    expect(getVisibleMutationActions(objectResource, writableServer, null)).toEqual({
      createRow: false,
      editRow: false,
      deleteRow: false,
      editObject: true
    });
  });
});

describe('loadOverviewPreferences', () => {
  it('restores saved local-storage preferences', () => {
    const storage = {
      getItem: () =>
        JSON.stringify({
          columnVisibility: {
            members: {
              team_id: false
            }
          },
          lastInspectorTab: 'schema',
          mobileSurface: 'map'
        })
    };

    expect(loadOverviewPreferences(storage)).toEqual({
      columnVisibility: {
        members: {
          team_id: false
        }
      },
      lastInspectorTab: 'schema',
      mobileSurface: 'map'
    });
  });
});
