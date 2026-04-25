import { describe, expect, it } from 'vitest';

import {
  deriveSchemaGraphRelations,
  deriveSchemaGraphTables,
  getSchemaGraphAutoLayout,
  mergeSchemaEditorPayload,
  removeDeclaredRelationship,
  resetDeclaredRelationship,
  setDeclaredColumnOverride,
  setDeclaredPrimaryKey,
  setDeclaredTableKind,
  upsertDeclaredRelationship,
  validateSchemaDraft
} from './schemaWorkspace';
import type { DeclaredSchemaResponse, ResourceOverview, SchemaResponse } from './types';

const inferredSchema: SchemaResponse = {
  tables: {
    users: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        team_id: { column_type: 'integer', nullable: true },
        name: { column_type: 'string', nullable: false }
      },
      foreign_keys: {
        team_id: {
          target_table: 'teams',
          target_column: 'id'
        }
      }
    },
    teams: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        name: { column_type: 'string', nullable: false }
      },
      foreign_keys: {}
    }
  }
};

const junctionSchema: SchemaResponse = {
  tables: {
    students: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        name: { column_type: 'string', nullable: false }
      },
      foreign_keys: {}
    },
    courses: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        title: { column_type: 'string', nullable: false }
      },
      foreign_keys: {}
    },
    student_courses: {
      kind: 'unknown',
      primary_key: null,
      columns: {
        student_id: { column_type: 'integer', nullable: false },
        course_id: { column_type: 'integer', nullable: false }
      },
      foreign_keys: {
        student_id: {
          target_table: 'students',
          target_column: 'id'
        },
        course_id: {
          target_table: 'courses',
          target_column: 'id'
        }
      }
    }
  }
};

const graphResources: ResourceOverview[] = [
  {
    name: 'users',
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: 'id',
    field_names: ['id', 'team_id', 'name'],
    row_samples: [],
    columns: [
      { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true },
      { name: 'team_id', column_type: 'integer', nullable: true, relation: 'foreign', is_primary_key: false },
      { name: 'name', column_type: 'string', nullable: false, relation: null, is_primary_key: false }
    ],
    outgoing_relations: [],
    incoming_relations: [],
    many_to_many_relations: [],
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
    name: 'teams',
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: 'id',
    field_names: ['id', 'name'],
    row_samples: [],
    columns: [
      { name: 'id', column_type: 'integer', nullable: false, relation: null, is_primary_key: true },
      { name: 'name', column_type: 'string', nullable: false, relation: null, is_primary_key: false }
    ],
    outgoing_relations: [],
    incoming_relations: [],
    many_to_many_relations: [],
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
    name: 'audit_logs',
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: null,
    field_names: ['message'],
    row_samples: [],
    columns: [{ name: 'message', column_type: 'string', nullable: false, relation: null, is_primary_key: false }],
    outgoing_relations: [],
    incoming_relations: [],
    many_to_many_relations: [],
    sample_item_id: null,
    query_capabilities: {
      filter: true,
      sort: true,
      pagination: true,
      embed: false,
      item_route: false
    },
    mutation_capabilities: {
      create_item: true,
      update_item: true,
      delete_item: true,
      replace_object: false,
      patch_object: false
    }
  }
];

const arrayIdSchema: SchemaResponse = {
  tables: {
    classes: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        title: { column_type: 'string', nullable: false },
        student_ids: { column_type: 'json', nullable: false }
      },
      foreign_keys: {}
    },
    students: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        name: { column_type: 'string', nullable: false }
      },
      foreign_keys: {}
    },
    teams: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        name: { column_type: 'string', nullable: false },
        memberIds: { column_type: 'json', nullable: false }
      },
      foreign_keys: {}
    },
    members: {
      kind: 'object',
      primary_key: 'id',
      columns: {
        id: { column_type: 'integer', nullable: false },
        handle: { column_type: 'string', nullable: false }
      },
      foreign_keys: {}
    }
  }
};

function buildGraphResource(
  name: string,
  fieldNames: string[],
  primaryKey: string | null
): ResourceOverview {
  return {
    name,
    kind: 'table',
    row_count: 2,
    key_count: null,
    primary_key: primaryKey,
    field_names: fieldNames,
    row_samples: [],
    columns: fieldNames.map((fieldName) => ({
      name: fieldName,
      column_type: fieldName === 'id' ? 'integer' : fieldName.endsWith('ids') || fieldName.endsWith('Ids') ? 'json' : 'string',
      nullable: false,
      relation: null,
      is_primary_key: fieldName === primaryKey
    })),
    outgoing_relations: [],
    incoming_relations: [],
    many_to_many_relations: [],
    sample_item_id: primaryKey ? '1' : null,
    query_capabilities: {
      filter: true,
      sort: true,
      pagination: true,
      embed: false,
      item_route: Boolean(primaryKey)
    },
    mutation_capabilities: {
      create_item: true,
      update_item: true,
      delete_item: true,
      replace_object: false,
      patch_object: false
    }
  };
}

const arrayIdResources: ResourceOverview[] = [
  buildGraphResource('classes', ['id', 'title', 'student_ids'], 'id'),
  buildGraphResource('students', ['id', 'name'], 'id'),
  buildGraphResource('teams', ['id', 'name', 'memberIds'], 'id'),
  buildGraphResource('members', ['id', 'handle'], 'id')
];

describe('schema workspace helpers', () => {
  it('merges inferred schema with declared overrides and suppressions', () => {
    const declared: DeclaredSchemaResponse = {
      tables: {
        users: {
          foreign_keys: {
            team_id: {
              target_table: 'teams',
              target_column: 'name'
            }
          },
          columns: {
            name: {
              column_type: 'json',
              nullable: true
            }
          }
        }
      }
    };

    const effective = mergeSchemaEditorPayload(inferredSchema, declared);

    expect(effective.tables?.users.columns?.name).toEqual({
      column_type: 'json',
      nullable: true
    });
    expect(effective.tables?.users.foreign_keys?.team_id).toEqual({
      target_table: 'teams',
      target_column: 'name'
    });
  });

  it('suppresses inferred relations without deleting other table metadata', () => {
    const declared: DeclaredSchemaResponse = {
      tables: {
        users: {
          suppressed_foreign_keys: ['team_id']
        }
      }
    };

    const effective = mergeSchemaEditorPayload(inferredSchema, declared);
    expect(effective.tables?.users.foreign_keys?.team_id).toBeUndefined();
    expect(effective.tables?.users.primary_key).toBe('id');
  });

  it('removes table and primary-key overrides when set back to automatic', () => {
    let declared: DeclaredSchemaResponse = { tables: {} };
    declared = setDeclaredTableKind(declared, 'users', 'relation');
    declared = setDeclaredPrimaryKey(declared, 'users', 'team_id');
    expect(declared.tables?.users.kind).toBe('relation');
    expect(declared.tables?.users.primary_key).toBe('team_id');

    declared = setDeclaredTableKind(declared, 'users', null);
    declared = setDeclaredPrimaryKey(declared, 'users', null);
    expect(declared.tables?.users).toBeUndefined();
  });

  it('drops column overrides when type and nullability return to automatic', () => {
    let declared: DeclaredSchemaResponse = { tables: {} };
    declared = setDeclaredColumnOverride(declared, inferredSchema, 'users', 'name', {
      columnType: 'json',
      nullable: true
    });
    expect(declared.tables?.users.columns?.name).toEqual({
      column_type: 'json',
      nullable: true
    });

    declared = setDeclaredColumnOverride(declared, inferredSchema, 'users', 'name', {
      columnType: null,
      nullable: null
    });
    expect(declared.tables?.users).toBeUndefined();
  });

  it('removes manual relations and suppresses inferred ones', () => {
    let declared: DeclaredSchemaResponse = { tables: {} };
    declared = upsertDeclaredRelationship(declared, {
      sourceTable: 'users',
      sourceColumn: 'team_id',
      targetTable: 'teams',
      targetColumn: 'name'
    });
    expect(declared.tables?.users.foreign_keys?.team_id?.target_column).toBe('name');

    declared = removeDeclaredRelationship(declared, inferredSchema, 'users', 'team_id');
    expect(declared.tables?.users.foreign_keys?.team_id).toBeUndefined();
    expect(declared.tables?.users.suppressed_foreign_keys).toEqual(['team_id']);
  });

  it('resets relations back to inferred behavior', () => {
    const declared = resetDeclaredRelationship(
      {
        tables: {
          users: {
            suppressed_foreign_keys: ['team_id']
          }
        }
      },
      'users',
      'team_id'
    );

    expect(declared.tables?.users).toBeUndefined();
  });

  it('validates relation targets and compatibility locally', () => {
    const invalidTarget = validateSchemaDraft(inferredSchema, {
      tables: {
        users: {
          foreign_keys: {
            team_id: {
              target_table: 'missing',
              target_column: 'id'
            }
          }
        }
      }
    });
    expect(invalidTarget).toContain("targets unknown table 'missing'");

    const incompatible = validateSchemaDraft(inferredSchema, {
      tables: {
        users: {
          foreign_keys: {
            team_id: {
              target_table: 'teams',
              target_column: 'name'
            }
          }
        }
      }
    });
    expect(incompatible).toContain("is incompatible with 'teams.name'");
  });

  it('derives strict junction kinds and many-to-many relations client-side', () => {
    const effective = mergeSchemaEditorPayload(junctionSchema, { tables: {} });

    expect(effective.tables?.student_courses.kind).toBe('relation');
    expect(effective.tables?.students.many_to_many?.courses).toEqual({
      through_table: 'student_courses',
      source_column: 'student_id',
      source_target_column: 'id',
      target_table: 'courses',
      target_column: 'id',
      through_target_column: 'course_id'
    });
    expect(effective.tables?.courses.many_to_many?.students?.through_table).toBe(
      'student_courses'
    );
  });

  it('keeps only compatible key columns visible in the schema graph', () => {
    const graphTables = deriveSchemaGraphTables(graphResources, inferredSchema.tables ?? {});

    expect(graphTables.users.columns.map((column) => column.name)).toEqual(['id', 'team_id']);
    expect(graphTables.users.columns.find((column) => column.name === 'team_id')).toMatchObject({
      canSource: true,
      canTarget: false
    });
    expect(graphTables.teams.columns).toEqual([
      expect.objectContaining({
        name: 'id',
        canSource: false,
        canTarget: true
      })
    ]);
    expect(graphTables.audit_logs.columns).toEqual([]);
  });

  it('derives one-to-many graph relations from _ids and Ids suffixes', () => {
    const relations = deriveSchemaGraphRelations(arrayIdResources, arrayIdSchema.tables ?? {});

    expect(relations).toEqual([
      {
        kind: 'one_to_many',
        sourceTable: 'classes',
        sourceColumn: 'student_ids',
        targetTable: 'students',
        targetColumn: 'id'
      },
      {
        kind: 'one_to_many',
        sourceTable: 'teams',
        sourceColumn: 'memberIds',
        targetTable: 'members',
        targetColumn: 'id'
      }
    ]);
  });

  it('shows inferred array-id columns in the schema graph without making them editable sources', () => {
    const graphTables = deriveSchemaGraphTables(arrayIdResources, arrayIdSchema.tables ?? {});

    expect(graphTables.classes.columns.find((column) => column.name === 'student_ids')).toMatchObject({
      relation: 'one_to_many',
      canSource: false,
      canTarget: false
    });
    expect(graphTables.students.columns).toEqual([
      expect.objectContaining({
        name: 'id',
        canTarget: true
      })
    ]);
    expect(graphTables.teams.columns.find((column) => column.name === 'memberIds')).toMatchObject({
      relation: 'one_to_many',
      canSource: false
    });
  });

  it('auto-arranges related tables from source to target order', () => {
    const layout = getSchemaGraphAutoLayout(graphResources, inferredSchema.tables ?? {});

    expect(layout.users.x).toBeLessThan(layout.teams.x);
    expect(layout.audit_logs.x).toBeGreaterThanOrEqual(0);
  });

  it('auto-arranges inferred array-id relations from source to target order', () => {
    const layout = getSchemaGraphAutoLayout(arrayIdResources, arrayIdSchema.tables ?? {});

    expect(layout.classes.x).toBeLessThan(layout.students.x);
    expect(layout.teams.x).toBeLessThan(layout.members.x);
  });
});
