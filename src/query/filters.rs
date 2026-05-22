#[allow(unused_imports)]
pub use super::evaluator::{
    filter_collection_data, filter_collection_refs, get_value_at_path, value_to_filter_string,
};
#[allow(unused_imports)]
pub use super::pagination::{
    paginate_collection_data, paginate_collection_refs, pagination_window,
};
pub use super::parser::parse_collection_query_params;
pub use super::sort::{sort_collection_data, sort_collection_refs};
#[allow(unused_imports)]
pub use super::types::{
    FilterCondition, FilterOperator, Pagination, PaginationWindow, ParsedCollectionQuery,
    SortColumn,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use crate::schema::{ColumnSchema, ColumnType, TableKind, TableSchema};

    use super::*;

    #[test]
    fn parses_where_like_json_server_cases() {
        let parsed = parse_collection_query_params(vec![
            ("views:gt".to_string(), "100".to_string()),
            ("title:eq".to_string(), "a".to_string()),
            ("views_lt".to_string(), "300".to_string()),
            ("first_name_eq".to_string(), "Alice".to_string()),
            ("author.first_name_ne".to_string(), "Bob".to_string()),
            ("title".to_string(), "hello".to_string()),
            ("id:in".to_string(), "1,3".to_string()),
            ("title:contains".to_string(), "ell".to_string()),
            ("title:startsWith".to_string(), "he".to_string()),
            ("title:endsWith".to_string(), "lo".to_string()),
        ])
        .expect("parse");

        let by_field = |name: &str| parsed.filters.iter().find(|f| f.field_path == name).unwrap();
        let view_operators = parsed
            .filters
            .iter()
            .filter(|f| f.field_path == "views")
            .map(|f| f.operator)
            .collect::<Vec<_>>();
        assert!(view_operators.contains(&FilterOperator::Gt));
        assert!(view_operators.contains(&FilterOperator::Lt));
        assert_eq!(by_field("title").operator, FilterOperator::Eq);
        assert_eq!(by_field("first_name").operator, FilterOperator::Eq);
        assert_eq!(by_field("author.first_name").operator, FilterOperator::Ne);
        assert_eq!(by_field("id").operator, FilterOperator::In);
    }

    #[test]
    fn ignores_unknown_underscore_suffix_as_plain_field() {
        let parsed = parse_collection_query_params(vec![
            ("views_foo".to_string(), "100".to_string()),
            ("title_eq".to_string(), "a".to_string()),
        ])
        .expect("parse");

        assert_eq!(parsed.filters[0].field_path, "views_foo");
        assert_eq!(parsed.filters[0].operator, FilterOperator::Eq);
        assert_eq!(parsed.filters[1].field_path, "title");
        assert_eq!(parsed.filters[1].operator, FilterOperator::Eq);
    }

    #[test]
    fn matches_where_like_operators() {
        let obj = json!({"a": 10, "b": 20, "c": "x", "nested": {"a": 10, "b": 20}});

        let cases = [
            (vec![("a:eq", "10")], true),
            (vec![("a:eq", "11")], false),
            (vec![("c:ne", "y")], true),
            (vec![("c:ne", "x")], false),
            (vec![("a:lt", "11")], true),
            (vec![("a:lt", "10")], false),
            (vec![("a:lte", "10")], true),
            (vec![("a:lte", "9")], false),
            (vec![("b:gt", "19")], true),
            (vec![("b:gt", "20")], false),
            (vec![("b:gte", "20")], true),
            (vec![("b:gte", "21")], false),
            (vec![("nested.a:eq", "10")], true),
            (vec![("nested.b:lt", "20")], false),
            (vec![("a:in", "10,11")], true),
            (vec![("a:in", "1,2")], false),
            (vec![("c:contains", "X")], true),
            (vec![("c:startsWith", "X")], true),
            (vec![("c:endsWith", "X")], true),
        ];

        for (filters, expected) in cases {
            let parsed = parse_collection_query_params(
                filters.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>(),
            )
            .expect("parse");
            let matches = filter_collection_data(json!([obj.clone()]), &parsed.filters, None)
                .expect("filter")
                .as_array()
                .expect("array")
                .len()
                == 1;
            assert_eq!(matches, expected, "filters: {filters:?}");
        }
    }

    #[test]
    fn operator_matrix_covers_nested_missing_null_and_string_cases() {
        let data = json!([
            {
                "id": 1,
                "name": "Ada Lovelace",
                "views": 10,
                "active": true,
                "deleted_at": null,
                "profile": {"city": "London"}
            },
            {
                "id": 2,
                "name": "Grace Hopper",
                "views": 20,
                "active": false,
                "profile": {"city": "Arlington"}
            },
            {
                "id": 3,
                "name": "Linus Torvalds",
                "views": "30",
                "active": "true",
                "deleted_at": "2026-01-01",
                "profile": {}
            }
        ]);

        let cases = [
            (vec![("id:eq", "1")], vec![1]),
            (vec![("id:ne", "1")], vec![2, 3]),
            (vec![("views:lt", "20")], vec![1]),
            (vec![("views:lte", "20")], vec![1, 2]),
            (vec![("views:gt", "20")], vec![3]),
            (vec![("views:gte", "20")], vec![2, 3]),
            (vec![("id:in", "1, 3")], vec![1, 3]),
            (vec![("name:contains", "HOP")], vec![2]),
            (vec![("name:startsWith", "ada")], vec![1]),
            (vec![("name:endsWith", "VALDS")], vec![3]),
            (vec![("deleted_at:isNull", "true")], vec![1, 2]),
            (vec![("deleted_at:isNotNull", "true")], vec![3]),
            (vec![("profile.city:eq", "London")], vec![1]),
            (vec![("profile.city:isNull", "true")], vec![3]),
            (vec![("active:eq", "true")], vec![1, 3]),
        ];

        for (filters, expected_ids) in cases {
            let parsed = parse_collection_query_params(
                filters.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            )
            .expect("parse filters");
            let filtered =
                filter_collection_data(data.clone(), &parsed.filters, None).expect("filter data");
            let actual_ids = filtered
                .as_array()
                .expect("array")
                .iter()
                .map(|item| item["id"].as_i64().expect("id"))
                .collect::<Vec<_>>();
            assert_eq!(actual_ids, expected_ids, "filters: {filters:?}");
        }
    }

    #[test]
    fn schema_aware_filtering_coerces_expected_values_by_column_type() {
        let data = json!([
            {"id": "1", "age": "10", "active": "true", "code": "010", "amount": "12.50"},
            {"id": "2", "age": "20", "active": "false", "code": "20", "amount": "100.25"}
        ]);
        let table = TableSchema {
            kind: TableKind::Unknown,
            primary_key: Some("id".to_string()),
            columns: BTreeMap::from([
                ("age".to_string(), ColumnSchema::new(ColumnType::Integer, false)),
                ("active".to_string(), ColumnSchema::new(ColumnType::Boolean, false)),
                ("code".to_string(), ColumnSchema::new(ColumnType::String, false)),
                ("amount".to_string(), ColumnSchema::new(ColumnType::Decimal, false)),
            ]),
            foreign_keys: BTreeMap::new(),
            many_to_many: BTreeMap::new(),
        };

        let parsed = parse_collection_query_params(vec![
            ("age:gte".to_string(), "10".to_string()),
            ("age:lt".to_string(), "20".to_string()),
            ("active:eq".to_string(), "true".to_string()),
            ("code:eq".to_string(), "010".to_string()),
            ("amount:lt".to_string(), "20".to_string()),
        ])
        .expect("parse filters");

        let filtered =
            filter_collection_data(data, &parsed.filters, Some(&table)).expect("filter data");
        assert_eq!(
            filtered,
            json!([{"id": "1", "age": "10", "active": "true", "code": "010", "amount": "12.50"}])
        );
    }

    #[test]
    fn paginates_like_json_server_boundaries() {
        let p1 =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 1, per_page: 2 })
                .expect("paginate");
        assert_eq!(p1["first"], 1);
        assert_eq!(p1["prev"], Value::Null);
        assert_eq!(p1["next"], 2);
        assert_eq!(p1["last"], 3);
        assert_eq!(p1["pages"], 3);
        assert_eq!(p1["items"], 5);
        assert_eq!(p1["data"], json!([1, 2]));

        let p2 =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 2, per_page: 2 })
                .expect("paginate");
        assert_eq!(p2["prev"], 1);
        assert_eq!(p2["next"], 3);
        assert_eq!(p2["data"], json!([3, 4]));

        let plast =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 9, per_page: 2 })
                .expect("paginate");
        assert_eq!(plast["prev"], 2);
        assert_eq!(plast["next"], Value::Null);
        assert_eq!(plast["data"], json!([5]));

        let p0 = paginate_collection_data(json!([1, 2, 3]), Pagination { page: 0, per_page: 2 })
            .expect("paginate");
        assert_eq!(p0["page"], 1);
        assert_eq!(p0["data"], json!([1, 2]));
    }
}
