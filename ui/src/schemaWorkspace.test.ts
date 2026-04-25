import { describe, expect, it } from 'vitest';

import {
  mergeSchemaEditorPayload,
  removeDeclaredRelationship,
  resetDeclaredRelationship,
  setDeclaredColumnOverride,
  setDeclaredPrimaryKey,
  setDeclaredTableKind,
  upsertDeclaredRelationship,
  validateSchemaDraft
} from './schemaWorkspace';
import type { DeclaredSchemaResponse, SchemaResponse } from './types';

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
});
