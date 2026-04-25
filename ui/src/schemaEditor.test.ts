import { describe, expect, it } from 'vitest';

import {
  buildSchemaHandleId,
  deriveSchemaEdges,
  parseSchemaConnection,
  parseSchemaDocument,
  upsertSchemaRelationship
} from './schemaEditor';
import type { ResourceOverview, SchemaResponse } from './types';

const postsResource: ResourceOverview = {
  name: 'posts',
  kind: 'table',
  row_count: 2,
  key_count: null,
  primary_key: 'id',
  field_names: ['id', 'author_id', 'title'],
  row_samples: [],
  columns: [
    {
      name: 'id',
      column_type: 'integer',
      nullable: false,
      relation: null,
      is_primary_key: true
    },
    {
      name: 'author_id',
      column_type: 'integer',
      nullable: false,
      relation: null,
      is_primary_key: false
    }
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
};

const usersResource: ResourceOverview = {
  ...postsResource,
  name: 'users',
  field_names: ['id', 'name'],
  columns: [
    {
      name: 'id',
      column_type: 'integer',
      nullable: false,
      relation: null,
      is_primary_key: true
    }
  ]
};

describe('schema editor helpers', () => {
  it('parses handle-based connections into schema relationships', () => {
    const connection = parseSchemaConnection({
      source: 'posts',
      target: 'users',
      sourceHandle: buildSchemaHandleId('source', 'author_id'),
      targetHandle: buildSchemaHandleId('target', 'id')
    });

    expect(connection).toEqual({
      sourceTable: 'posts',
      sourceColumn: 'author_id',
      targetTable: 'users',
      targetColumn: 'id'
    });
  });

  it('upserts foreign keys into the schema draft', () => {
    const schema: SchemaResponse = {
      tables: {
        posts: {
          foreign_keys: {}
        }
      }
    };

    const nextSchema = upsertSchemaRelationship(schema, {
      sourceTable: 'posts',
      sourceColumn: 'author_id',
      targetTable: 'users',
      targetColumn: 'id'
    });

    expect(nextSchema.tables?.posts.foreign_keys).toEqual({
      author_id: {
        target_table: 'users',
        target_column: 'id'
      }
    });
  });

  it('derives edges from the current schema draft', () => {
    const edges = deriveSchemaEdges(
      {
        tables: {
          posts: {
            foreign_keys: {
              author_id: {
                target_table: 'users',
                target_column: 'id'
              }
            }
          }
        }
      },
      [postsResource, usersResource]
    );

    expect(edges).toEqual([
      {
        source_table: 'posts',
        source_column: 'author_id',
        target_table: 'users',
        target_column: 'id'
      }
    ]);
  });

  it('reports invalid JSON drafts', () => {
    const result = parseSchemaDocument('{');

    expect(result.document).toBeNull();
    expect(result.error).toContain('Expected');
  });
});
