use super::*;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use serde_json::json;

#[test]
fn parses_dbml_schema() {
    let schema = parse_dbml_schema(
        r#"
            Table users {
              id int [pk]
              name varchar [not null]
              active bool
            }
            "#,
    )
    .expect("parse schema");

    let users = schema.tables.get("users").expect("users table");
    assert_eq!(users.columns["id"].column_type, ColumnType::Integer);
    assert_eq!(users.columns["name"].column_type, ColumnType::String);
    assert_eq!(users.columns["active"].column_type, ColumnType::Boolean);
    assert!(!users.columns["id"].nullable);
    assert_eq!(users.kind, Some(TableKind::Object));
    assert_eq!(users.primary_key.as_deref(), Some("id"));
}

#[test]
fn parses_extended_dbml_column_types() {
    let schema = parse_dbml_schema(
        r#"
            Table events {
              id uuid [pk]
              starts_on date
              starts_at timestamptz
              amount numeric
              counter bigint
            }
            "#,
    )
    .expect("parse schema");

    let events = schema.tables.get("events").expect("events table");
    assert_eq!(events.columns["id"].column_type, ColumnType::Uuid);
    assert_eq!(events.columns["starts_on"].column_type, ColumnType::Date);
    assert_eq!(events.columns["starts_at"].column_type, ColumnType::DateTime);
    assert_eq!(events.columns["amount"].column_type, ColumnType::Decimal);
    assert_eq!(events.columns["counter"].column_type, ColumnType::BigInteger);
}

#[test]
fn column_types_use_stable_wire_names() {
    let cases = [
        (ColumnType::Integer, "integer"),
        (ColumnType::Float, "float"),
        (ColumnType::Boolean, "boolean"),
        (ColumnType::String, "string"),
        (ColumnType::Json, "json"),
        (ColumnType::Date, "date"),
        (ColumnType::DateTime, "datetime"),
        (ColumnType::Uuid, "uuid"),
        (ColumnType::BigInteger, "big_integer"),
        (ColumnType::Decimal, "decimal"),
    ];

    for (column_type, wire_name) in cases {
        assert_eq!(column_type.label(), wire_name);
        assert_eq!(
            serde_json::to_value(&column_type).expect("serialize type"),
            Value::from(wire_name)
        );
        assert_eq!(
            serde_json::from_value::<ColumnType>(Value::from(wire_name)).expect("deserialize type"),
            column_type
        );
    }
}

#[test]
fn maps_dbml_type_aliases_and_parameterized_types() {
    let cases = [
        ("smallint", ColumnType::Integer),
        ("serial", ColumnType::Integer),
        ("bigserial", ColumnType::BigInteger),
        ("double", ColumnType::Float),
        ("real", ColumnType::Float),
        ("numeric(20,6)", ColumnType::Decimal),
        ("decimal(10,2)", ColumnType::Decimal),
        ("boolean", ColumnType::Boolean),
        ("jsonb", ColumnType::Json),
        ("timestamp", ColumnType::DateTime),
        ("timestamptz", ColumnType::DateTime),
        ("varchar(255)", ColumnType::String),
    ];

    for (raw_type, expected) in cases {
        assert_eq!(ColumnType::from_dbml_type(raw_type), expected, "{raw_type}");
    }
}

#[test]
fn numeric_schema_types_are_foreign_key_compatible() {
    let numeric_types =
        [ColumnType::Integer, ColumnType::Float, ColumnType::BigInteger, ColumnType::Decimal];

    for left in &numeric_types {
        for right in &numeric_types {
            assert!(left.is_compatible_with(right), "{left:?} should accept {right:?}");
        }
    }

    assert!(ColumnType::Uuid.is_compatible_with(&ColumnType::Uuid));
    assert!(!ColumnType::Uuid.is_compatible_with(&ColumnType::String));
    assert!(!ColumnType::Boolean.is_compatible_with(&ColumnType::Integer));
    assert!(!ColumnType::Date.is_compatible_with(&ColumnType::DateTime));
}

#[test]
fn validates_declared_column_constraints() {
    let inferred = infer_schema_from_values(&BTreeMap::from([(
        "posts".to_string(),
        json!([{"id": 1, "status": "draft", "slug": "hello", "score": 3, "published_on": "2026-04-29"}]),
    )]));
    let mut status = ColumnSchema::new(ColumnType::String, false);
    status.enum_values = Some(vec!["draft".to_string(), "published".to_string()]);
    let mut slug = ColumnSchema::new(ColumnType::String, false);
    slug.min_length = Some(3);
    slug.max_length = Some(20);
    slug.pattern = Some("^[a-z0-9-]+$".to_string());
    let mut score = ColumnSchema::new(ColumnType::Integer, false);
    score.min = Some(Value::from(1));
    score.max = Some(Value::from(5));
    let mut published_on = ColumnSchema::new(ColumnType::Date, false);
    published_on.min = Some(Value::from("2026-01-01"));
    published_on.max = Some(Value::from("2026-12-31"));

    let declared = DeclaredSchema {
        tables: BTreeMap::from([(
            "posts".to_string(),
            DeclaredTableSchema {
                columns: BTreeMap::from([
                    ("status".to_string(), status),
                    ("slug".to_string(), slug),
                    ("score".to_string(), score),
                    ("published_on".to_string(), published_on),
                ]),
                unique: vec![vec!["slug".to_string()]],
                ..DeclaredTableSchema::default()
            },
        )]),
    };

    merge_schemas(Some(&declared), &inferred).expect("valid constraints");
}

#[test]
fn rejects_invalid_declared_constraints() {
    let inferred = infer_schema_from_values(&BTreeMap::from([(
        "posts".to_string(),
        json!([{"id": 1, "status": "draft"}]),
    )]));
    let mut status = ColumnSchema::new(ColumnType::String, false);
    status.enum_values = Some(vec!["draft".to_string(), "draft".to_string()]);
    let declared = DeclaredSchema {
        tables: BTreeMap::from([(
            "posts".to_string(),
            DeclaredTableSchema {
                columns: BTreeMap::from([("status".to_string(), status)]),
                ..DeclaredTableSchema::default()
            },
        )]),
    };

    let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid constraints");
    assert!(err.contains("duplicate enum value"));
}

#[test]
fn rejects_type_specific_constraint_mismatches() {
    let inferred = infer_schema_from_values(&BTreeMap::from([(
        "posts".to_string(),
        json!([{"id": 1, "title": "Hello", "metadata": {}}]),
    )]));

    fn expect_constraint_error(
        inferred: &Schema,
        column_name: &str,
        column: ColumnSchema,
        expected: &str,
    ) {
        let declared = DeclaredSchema {
            tables: BTreeMap::from([(
                "posts".to_string(),
                DeclaredTableSchema {
                    columns: BTreeMap::from([(column_name.to_string(), column)]),
                    ..DeclaredTableSchema::default()
                },
            )]),
        };

        let err = merge_schemas(Some(&declared), inferred).expect_err("invalid constraint");
        assert!(err.contains(expected), "{err}");
    }

    let mut uuid = ColumnSchema::new(ColumnType::Uuid, false);
    uuid.min = Some(Value::from("00000000-0000-0000-0000-000000000000"));
    expect_constraint_error(&inferred, "id", uuid, "min/max on unsupported type 'uuid'");

    let mut metadata = ColumnSchema::new(ColumnType::Json, true);
    metadata.pattern = Some(".*".to_string());
    expect_constraint_error(&inferred, "metadata", metadata, "pattern on unsupported type 'json'");

    let mut date = ColumnSchema::new(ColumnType::Date, false);
    date.min = Some(Value::from("2026-12-31"));
    date.max = Some(Value::from("2026-01-01"));
    expect_constraint_error(&inferred, "published_on", date, "min greater than max");

    let mut timestamp = ColumnSchema::new(ColumnType::DateTime, false);
    timestamp.min = Some(Value::from("not-a-date-time"));
    expect_constraint_error(&inferred, "published_at", timestamp, "declares invalid min");
}

#[test]
fn rejects_invalid_unique_constraints() {
    let inferred = infer_schema_from_values(&BTreeMap::from([(
        "posts".to_string(),
        json!([{"id": 1, "slug": "hello"}]),
    )]));
    let declared = DeclaredSchema {
        tables: BTreeMap::from([(
            "posts".to_string(),
            DeclaredTableSchema {
                unique: vec![vec!["missing".to_string()]],
                ..DeclaredTableSchema::default()
            },
        )]),
    };

    let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid unique");
    assert!(err.contains("unique constraint references unknown column 'missing'"));
}

#[test]
fn rejects_invalid_identifiers() {
    let err = parse_dbml_schema(
        r#"
            Table users {
              bad$name int
            }
            "#,
    )
    .expect_err("invalid schema");
    assert!(err.contains("invalid column name"));
}

#[test]
fn parses_inline_and_top_level_refs() {
    let schema = parse_dbml_schema(
        r#"
            Table users {
              id int [pk]
              name varchar
            }

            Table posts {
              id int [pk]
              user_id int [ref: > users.id]
            }

            Ref: posts.user_id > users.id
            "#,
    )
    .expect("parse schema");

    let posts = schema.tables.get("posts").expect("posts table");
    let fk = posts.foreign_keys.get("user_id").expect("user_id foreign key");
    assert_eq!(fk.target_table, "users");
    assert_eq!(fk.target_column, "id");
}

#[test]
fn infers_object_and_relation_tables() {
    let schema = infer_schema_from_values(&BTreeMap::from([
        (
            "students".to_string(),
            json!([
                {"id": 1, "name": "Ada"},
                {"id": 2, "name": "Grace"}
            ]),
        ),
        (
            "courses".to_string(),
            json!([
                {"id": 10, "title": "Math"},
                {"id": 11, "title": "CS"}
            ]),
        ),
        (
            "student_courses".to_string(),
            json!([
                {"student_id": 1, "course_id": 10},
                {"student_id": 2, "course_id": 11}
            ]),
        ),
    ]));

    let students = schema.tables.get("students").expect("students table");
    assert_eq!(students.kind, TableKind::Object);
    assert_eq!(students.primary_key.as_deref(), Some("id"));

    let relation = schema.tables.get("student_courses").expect("relation table");
    assert_eq!(relation.kind, TableKind::Relation);
    assert_eq!(relation.foreign_keys["student_id"].target_table, "students");
    assert_eq!(relation.foreign_keys["course_id"].target_table, "courses");
    assert_eq!(students.many_to_many["courses"].through_table, "student_courses");
    assert_eq!(students.many_to_many["courses"].source_column, "student_id");
    assert_eq!(students.many_to_many["courses"].through_target_column, "course_id");
    assert_eq!(schema.tables["courses"].many_to_many["students"].through_table, "student_courses");
}

#[test]
fn strict_junction_detection_avoids_false_positives() {
    let schema = infer_schema_from_values(&BTreeMap::from([
        (
            "students".to_string(),
            json!([
                {"id": 1, "name": "Ada"},
                {"id": 2, "name": "Grace"}
            ]),
        ),
        (
            "courses".to_string(),
            json!([
                {"id": 10, "title": "Math"},
                {"id": 11, "title": "CS"}
            ]),
        ),
        (
            "student_courses".to_string(),
            json!([
                {"student_id": 1, "course_id": 10, "role": "lead"},
                {"student_id": 2, "course_id": 11, "role": "assistant"}
            ]),
        ),
        (
            "labels".to_string(),
            json!([
                {"student_id": 1, "label": "mentor"},
                {"student_id": 2, "label": "helper"}
            ]),
        ),
    ]));

    assert_eq!(schema.tables["student_courses"].kind, TableKind::Unknown);
    assert!(schema.tables["student_courses"].many_to_many.is_empty());
    assert_eq!(schema.tables["labels"].kind, TableKind::Unknown);
    assert!(schema.tables["labels"].foreign_keys.contains_key("student_id"));
    assert!(schema.tables["labels"].many_to_many.is_empty());
}

#[test]
fn infers_non_id_primary_keys() {
    let schema = infer_schema_from_values(&BTreeMap::from([(
        "users".to_string(),
        json!([
            {"user_id": 1, "name": "Ada"},
            {"user_id": 2, "name": "Grace"}
        ]),
    )]));

    assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("user_id"));
}

#[test]
fn merges_declared_foreign_key_over_inferred_data() {
    let inferred = infer_schema_from_values(&BTreeMap::from([
        (
            "users".to_string(),
            json!([
                {"user_id": 1, "name": "Ada"},
                {"user_id": 2, "name": "Grace"}
            ]),
        ),
        (
            "posts".to_string(),
            json!([
                {"id": 1, "author_id": 1}
            ]),
        ),
    ]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
    let posts = merged.tables.get("posts").expect("posts table");
    assert_eq!(posts.foreign_keys["author_id"].target_column, "user_id");
}

#[test]
fn partial_declared_schema_preserves_inferred_columns() {
    let inferred = infer_schema_from_values(&BTreeMap::from([
        ("users".to_string(), json!([{"user_id": 1, "name": "Ada"}])),
        (
            "posts".to_string(),
            json!([
                {"id": 1, "author_id": 1, "title": "Hello"}
            ]),
        ),
    ]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
    let posts = merged.tables.get("posts").expect("posts table");
    assert!(posts.columns.contains_key("title"));
    assert!(posts.columns.contains_key("author_id"));
}

#[test]
fn suppressed_foreign_keys_remove_inferred_relations() {
    let inferred = infer_schema_from_values(&BTreeMap::from([
        ("users".to_string(), json!([{"id": 1, "name": "Ada"}])),
        ("posts".to_string(), json!([{"id": 1, "user_id": 1}])),
    ]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "posts": {
                  "suppressed_foreign_keys": ["user_id"]
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
    let posts = merged.tables.get("posts").expect("posts table");
    assert!(!posts.foreign_keys.contains_key("user_id"), "{posts:?}");
}

#[test]
fn export_declared_snapshot_preserves_suppressed_foreign_keys() {
    let effective = infer_schema_from_values(&BTreeMap::from([
        ("users".to_string(), json!([{"id": 1, "name": "Ada"}])),
        ("posts".to_string(), json!([{"id": 1, "user_id": 1}])),
    ]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "posts": {
                  "suppressed_foreign_keys": ["user_id"]
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let snapshot = export_declared_schema_snapshot(Some(&declared), &effective);
    assert_eq!(
        snapshot.tables["posts"].suppressed_foreign_keys,
        BTreeSet::from(["user_id".to_string()])
    );
}

#[test]
fn parses_schema_json() {
    let raw = r#"
        {
          "tables": {
            "users": {
              "kind": "object",
              "primary_key": "id",
              "columns": {
                "id": {"column_type": "integer", "nullable": false}
              },
              "foreign_keys": {}
            }
          }
        }
        "#;

    let schema = parse_json_schema(raw).expect("parse schema json");
    assert_eq!(schema.tables["users"].kind, Some(TableKind::Object));
    assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("id"));
}

#[test]
fn parses_schema_xsd_tables_keys_and_keyrefs() {
    let raw = r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="users">
            <xs:complexType>
              <xs:sequence>
                <xs:element name="user_id" type="xs:int"/>
                <xs:element name="name" type="xs:string" minOccurs="0"/>
              </xs:sequence>
            </xs:complexType>
            <xs:key name="users_pk">
              <xs:selector xpath="."/>
              <xs:field xpath="user_id"/>
            </xs:key>
          </xs:element>
          <xs:element name="posts">
            <xs:complexType>
              <xs:sequence>
                <xs:element name="id" type="xs:int"/>
                <xs:element name="author_ref" type="xs:int"/>
                <xs:element name="title" type="xs:string"/>
              </xs:sequence>
            </xs:complexType>
            <xs:key name="posts_pk">
              <xs:selector xpath="."/>
              <xs:field xpath="id"/>
            </xs:key>
            <xs:keyref name="posts_author_ref_fk" refer="users_pk">
              <xs:selector xpath="."/>
              <xs:field xpath="author_ref"/>
            </xs:keyref>
          </xs:element>
        </xs:schema>
        "#;

    let schema = parse_xsd_schema(raw).expect("parse schema xsd");
    assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("user_id"));
    assert_eq!(schema.tables["users"].kind, Some(TableKind::Object));
    assert_eq!(schema.tables["users"].columns["name"].column_type, ColumnType::String);
    assert!(schema.tables["users"].columns["name"].nullable);
    assert_eq!(
        schema.tables["posts"].foreign_keys["author_ref"],
        ForeignKey { target_table: "users".to_string(), target_column: "user_id".to_string() }
    );
}

#[test]
fn parses_schema_xsd_wrapped_repeating_tables() {
    let raw = r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="database">
            <xs:complexType>
              <xs:sequence>
                <xs:element name="users" maxOccurs="unbounded">
                  <xs:complexType>
                    <xs:sequence>
                      <xs:element name="id" type="xs:int"/>
                      <xs:element name="role" minOccurs="0">
                        <xs:simpleType>
                          <xs:restriction base="xs:string">
                            <xs:enumeration value="admin"/>
                            <xs:enumeration value="member"/>
                          </xs:restriction>
                        </xs:simpleType>
                      </xs:element>
                    </xs:sequence>
                  </xs:complexType>
                </xs:element>
              </xs:sequence>
            </xs:complexType>
            <xs:key name="users_pk">
              <xs:selector xpath="users"/>
              <xs:field xpath="id"/>
            </xs:key>
          </xs:element>
        </xs:schema>
        "#;

    let schema = parse_xsd_schema(raw).expect("parse schema xsd");
    assert!(schema.tables.contains_key("users"));
    assert!(!schema.tables.contains_key("database"));
    assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("id"));
    assert_eq!(
        schema.tables["users"].columns["role"].enum_values.as_ref().expect("enum values"),
        &vec!["admin".to_string(), "member".to_string()]
    );
}

#[test]
fn parses_schema_xsd_named_complex_and_simple_types() {
    let raw = r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:simpleType name="RoleType">
            <xs:restriction base="xs:string">
              <xs:enumeration value="admin"/>
              <xs:enumeration value="member"/>
            </xs:restriction>
          </xs:simpleType>
          <xs:complexType name="UserRow">
            <xs:sequence>
              <xs:element name="id" type="xs:int"/>
              <xs:element name="role" type="RoleType"/>
            </xs:sequence>
          </xs:complexType>
          <xs:element name="users" type="UserRow">
            <xs:key name="users_pk">
              <xs:selector xpath="."/>
              <xs:field xpath="id"/>
            </xs:key>
          </xs:element>
        </xs:schema>
        "#;

    let schema = parse_xsd_schema(raw).expect("parse schema xsd");
    assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("id"));
    assert_eq!(schema.tables["users"].columns["id"].column_type, ColumnType::Integer);
    assert_eq!(
        schema.tables["users"].columns["role"].enum_values.as_ref().expect("enum values"),
        &vec!["admin".to_string(), "member".to_string()]
    );
}

#[test]
fn validates_foreign_key_targets() {
    let inferred = infer_schema_from_values(&BTreeMap::from([(
        "posts".to_string(),
        json!([{"id": 1, "author_id": 1}]),
    )]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "id"}
                  }
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid fk");
    assert!(err.contains("targets unknown table"));
}

#[test]
fn validates_foreign_key_type_compatibility() {
    let inferred = infer_schema_from_values(&BTreeMap::from([
        ("users".to_string(), json!([{"user_id": "user-1"}])),
        ("posts".to_string(), json!([{"author_id": 1}])),
    ]));
    let declared = parse_json_schema(
        r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
    )
    .expect("parse schema");

    let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid fk type");
    assert!(err.contains("incompatible"));
}

#[test]
fn dbml_and_json_yield_same_effective_schema() {
    let inferred = infer_schema_from_values(&BTreeMap::from([
        (
            "users".to_string(),
            json!([
                {"user_id": 1, "name": "Ada"},
                {"user_id": 2, "name": "Grace"}
            ]),
        ),
        (
            "posts".to_string(),
            json!([
                {"id": 1, "author_ref": 1, "title": "Hello"}
            ]),
        ),
    ]));
    let dbml = parse_dbml_schema(
        r#"
            Table users {
              user_id int [pk]
              name varchar
            }

            Table posts {
              id int [pk]
              author_ref int
            }

            Ref: posts.author_ref > users.user_id
            "#,
    )
    .expect("parse dbml");
    let json = parse_json_schema(
        r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_ref": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
    )
    .expect("parse json");

    let from_dbml = merge_schemas(Some(&dbml), &inferred).expect("merge dbml");
    let from_json = merge_schemas(Some(&json), &inferred).expect("merge json");
    assert_eq!(from_dbml.tables["users"].primary_key, from_json.tables["users"].primary_key);
    assert_eq!(from_dbml.tables["posts"].foreign_keys, from_json.tables["posts"].foreign_keys);
}
