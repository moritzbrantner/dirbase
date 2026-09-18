use serde_json::Value;

use crate::schema::TableSchema;

use super::{
    evaluator::item_matches_filters,
    pagination::pagination_window,
    sort::{compare_items_by_columns, sort_collection_refs},
    types::{FilterCondition, Pagination, PaginationWindow, SortColumn},
};

const BOUNDED_SORT_PREFIX_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionExecutionPlan {
    DirectWindow,
    FilteredWindow,
    BoundedSortedWindow,
    FullMaterialization,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectionExecutionStats {
    pub source_rows_visited: usize,
    pub matched_rows: usize,
    pub sort_candidates_retained: usize,
    pub output_rows: usize,
}

pub struct CollectionExecutionResult<'a> {
    pub plan: CollectionExecutionPlan,
    pub rows: Vec<&'a Value>,
    pub pagination: Option<PaginationWindow>,
    pub stats: CollectionExecutionStats,
}

#[derive(Clone, Copy)]
struct IndexedRow<'a> {
    source_index: usize,
    value: &'a Value,
}

pub fn execute_collection_query<'a>(
    items: &'a [Value],
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
    pagination: Option<Pagination>,
    table: Option<&TableSchema>,
) -> CollectionExecutionResult<'a> {
    let plan = compile_collection_execution_plan(
        items.len(),
        filters,
        sort_columns,
        pagination,
    );
    match plan {
        CollectionExecutionPlan::DirectWindow => {
            direct_window(items, pagination.expect("direct window requires pagination"))
        }
        CollectionExecutionPlan::FilteredWindow => filtered_window(
            items,
            filters,
            pagination.expect("filtered window requires pagination"),
            table,
        ),
        CollectionExecutionPlan::BoundedSortedWindow => bounded_sorted_window(
            items,
            filters,
            sort_columns,
            pagination.expect("bounded sort requires pagination"),
            table,
        ),
        CollectionExecutionPlan::FullMaterialization => full_materialization(
            items,
            filters,
            sort_columns,
            pagination,
            table,
        ),
    }
}

pub fn materialize_collection_result(result: CollectionExecutionResult<'_>) -> Value {
    let data = result
        .rows
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let Some(window) = result.pagination else {
        return Value::Array(data);
    };

    serde_json::json!({
        "first": window.first,
        "prev": window.prev,
        "next": window.next,
        "last": window.last,
        "page": window.page,
        "pages": window.pages,
        "items": window.items,
        "data": data,
    })
}

fn compile_collection_execution_plan(
    source_len: usize,
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
    pagination: Option<Pagination>,
) -> CollectionExecutionPlan {
    let Some(pagination) = pagination else {
        return CollectionExecutionPlan::FullMaterialization;
    };
    if filters.is_empty() && sort_columns.is_empty() {
        return CollectionExecutionPlan::DirectWindow;
    }
    if sort_columns.is_empty() {
        return CollectionExecutionPlan::FilteredWindow;
    }

    let requested_page = pagination.page.max(1);
    let requested_end = requested_page.saturating_mul(pagination.per_page);
    if requested_end <= BOUNDED_SORT_PREFIX_LIMIT && requested_end <= source_len {
        CollectionExecutionPlan::BoundedSortedWindow
    } else {
        CollectionExecutionPlan::FullMaterialization
    }
}

fn direct_window(items: &[Value], pagination: Pagination) -> CollectionExecutionResult<'_> {
    let window = pagination_window(items.len(), pagination);
    let rows = items[window.start..window.end].iter().collect::<Vec<_>>();
    CollectionExecutionResult {
        plan: CollectionExecutionPlan::DirectWindow,
        stats: CollectionExecutionStats {
            source_rows_visited: rows.len(),
            matched_rows: items.len(),
            sort_candidates_retained: 0,
            output_rows: rows.len(),
        },
        rows,
        pagination: Some(window),
    }
}

fn filtered_window<'a>(
    items: &'a [Value],
    filters: &[FilterCondition],
    pagination: Pagination,
    table: Option<&TableSchema>,
) -> CollectionExecutionResult<'a> {
    let requested_page = pagination.page.max(1);
    let requested_start = requested_page
        .saturating_sub(1)
        .saturating_mul(pagination.per_page);
    let requested_end = requested_start.saturating_add(pagination.per_page);
    let mut requested = Vec::with_capacity(pagination.per_page);
    let mut trailing = Vec::with_capacity(pagination.per_page);
    let mut matched = 0usize;

    for item in items {
        if !item_matches_filters(item, filters, table) {
            continue;
        }
        if matched >= requested_start && matched < requested_end {
            requested.push(item);
        }
        trailing.push(item);
        if trailing.len() > pagination.per_page {
            trailing.remove(0);
        }
        matched += 1;
    }

    let window = pagination_window(matched, pagination);
    let rows = if window.page == requested_page {
        requested
    } else {
        let page_len = window.end.saturating_sub(window.start);
        trailing
            .into_iter()
            .skip(pagination.per_page.saturating_sub(page_len))
            .collect()
    };

    CollectionExecutionResult {
        plan: CollectionExecutionPlan::FilteredWindow,
        stats: CollectionExecutionStats {
            source_rows_visited: items.len(),
            matched_rows: matched,
            sort_candidates_retained: 0,
            output_rows: rows.len(),
        },
        rows,
        pagination: Some(window),
    }
}

fn bounded_sorted_window<'a>(
    items: &'a [Value],
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
    pagination: Pagination,
    table: Option<&TableSchema>,
) -> CollectionExecutionResult<'a> {
    let requested_page = pagination.page.max(1);
    let requested_end = requested_page.saturating_mul(pagination.per_page);
    let requested_start = requested_end.saturating_sub(pagination.per_page);
    let mut prefix = Vec::with_capacity(requested_end);
    let mut suffix = Vec::with_capacity(pagination.per_page);
    let mut matched = 0usize;

    for (source_index, item) in items.iter().enumerate() {
        if !filters.is_empty() && !item_matches_filters(item, filters, table) {
            continue;
        }
        let candidate = IndexedRow { source_index, value: item };
        retain_best(&mut prefix, candidate, requested_end, sort_columns);
        retain_worst(&mut suffix, candidate, pagination.per_page, sort_columns);
        matched += 1;
    }

    let window = pagination_window(matched, pagination);
    let rows = if window.page == requested_page {
        prefix[window.start..window.end]
            .iter()
            .map(|row| row.value)
            .collect()
    } else {
        let page_len = window.end.saturating_sub(window.start);
        suffix
            .into_iter()
            .skip(pagination.per_page.saturating_sub(page_len))
            .map(|row| row.value)
            .collect()
    };

    CollectionExecutionResult {
        plan: CollectionExecutionPlan::BoundedSortedWindow,
        stats: CollectionExecutionStats {
            source_rows_visited: items.len(),
            matched_rows: matched,
            sort_candidates_retained: prefix.len().saturating_add(suffix.len()),
            output_rows: rows.len(),
        },
        rows,
        pagination: Some(window),
    }
}

fn full_materialization<'a>(
    items: &'a [Value],
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
    pagination: Option<Pagination>,
    table: Option<&TableSchema>,
) -> CollectionExecutionResult<'a> {
    let mut selected = if filters.is_empty() {
        items.iter().collect::<Vec<_>>()
    } else {
        items
            .iter()
            .filter(|item| item_matches_filters(item, filters, table))
            .collect::<Vec<_>>()
    };
    let matched = selected.len();
    if !sort_columns.is_empty() {
        sort_collection_refs(selected.as_mut_slice(), sort_columns);
    }

    let (rows, window) = if let Some(pagination) = pagination {
        let window = pagination_window(matched, pagination);
        (selected[window.start..window.end].to_vec(), Some(window))
    } else {
        (selected, None)
    };
    let retained = if sort_columns.is_empty() { 0 } else { matched };

    CollectionExecutionResult {
        plan: CollectionExecutionPlan::FullMaterialization,
        stats: CollectionExecutionStats {
            source_rows_visited: items.len(),
            matched_rows: matched,
            sort_candidates_retained: retained,
            output_rows: rows.len(),
        },
        rows,
        pagination: window,
    }
}

fn compare_indexed(
    left: &IndexedRow<'_>,
    right: &IndexedRow<'_>,
    sort_columns: &[SortColumn],
) -> std::cmp::Ordering {
    compare_items_by_columns(left.value, right.value, sort_columns)
        .then_with(|| left.source_index.cmp(&right.source_index))
}

fn retain_best<'a>(
    retained: &mut Vec<IndexedRow<'a>>,
    candidate: IndexedRow<'a>,
    limit: usize,
    sort_columns: &[SortColumn],
) {
    if limit == 0 {
        return;
    }
    let position = retained
        .binary_search_by(|existing| compare_indexed(existing, &candidate, sort_columns))
        .unwrap_or_else(|position| position);
    if retained.len() == limit && position >= limit {
        return;
    }
    retained.insert(position, candidate);
    if retained.len() > limit {
        retained.pop();
    }
}

fn retain_worst<'a>(
    retained: &mut Vec<IndexedRow<'a>>,
    candidate: IndexedRow<'a>,
    limit: usize,
    sort_columns: &[SortColumn],
) {
    if limit == 0 {
        return;
    }
    let position = retained
        .binary_search_by(|existing| compare_indexed(existing, &candidate, sort_columns))
        .unwrap_or_else(|position| position);
    if retained.len() == limit && position == 0 {
        return;
    }
    retained.insert(position, candidate);
    if retained.len() > limit {
        retained.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::query::{
        evaluator::filter_collection_refs,
        pagination::paginate_collection_refs,
        sort::sort_collection_refs,
        types::{FilterOperator, SortColumn},
    };

    fn baseline(
        data: &[Value],
        filters: &[FilterCondition],
        sort_columns: &[SortColumn],
        pagination: Pagination,
    ) -> Value {
        let mut selected = if filters.is_empty() {
            data.iter().collect::<Vec<_>>()
        } else {
            filter_collection_refs(data, filters, None)
        };
        if !sort_columns.is_empty() {
            sort_collection_refs(&mut selected, sort_columns);
        }
        paginate_collection_refs(&selected, pagination)
    }

    #[test]
    fn direct_window_does_not_visit_unrequested_rows() {
        let data = (1..=100).map(|id| json!({"id": id})).collect::<Vec<_>>();
        let result = execute_collection_query(
            &data,
            &[],
            &[],
            Some(Pagination { page: 2, per_page: 3 }),
            None,
        );

        assert_eq!(result.plan, CollectionExecutionPlan::DirectWindow);
        assert_eq!(result.stats.source_rows_visited, 3);
        assert_eq!(result.stats.output_rows, 3);
        assert_eq!(
            materialize_collection_result(result)["data"],
            json!([{"id": 4}, {"id": 5}, {"id": 6}])
        );
    }

    #[test]
    fn filtered_window_matches_full_pipeline_and_clamps_late_page() {
        let data = (1..=20)
            .map(|id| json!({"id": id, "group": id % 3}))
            .collect::<Vec<_>>();
        let filters = vec![FilterCondition::new(
            "group".to_string(),
            FilterOperator::Eq,
            "1".to_string(),
        )];
        let pagination = Pagination { page: 99, per_page: 3 };

        let result = execute_collection_query(&data, &filters, &[], Some(pagination), None);
        assert_eq!(result.plan, CollectionExecutionPlan::FilteredWindow);
        assert_eq!(
            materialize_collection_result(result),
            baseline(&data, &filters, &[], pagination)
        );
    }

    #[test]
    fn bounded_sorted_window_matches_stable_full_sort_with_ties() {
        let data = (1..=80)
            .map(|id| json!({"id": id, "group": id % 4, "rank": id % 7}))
            .collect::<Vec<_>>();
        let filters = vec![FilterCondition::new(
            "group".to_string(),
            FilterOperator::Eq,
            "2".to_string(),
        )];
        let sort = vec![SortColumn { field_path: "rank".to_string(), descending: true }];
        let pagination = Pagination { page: 2, per_page: 5 };

        let result =
            execute_collection_query(&data, &filters, &sort, Some(pagination), None);
        assert_eq!(result.plan, CollectionExecutionPlan::BoundedSortedWindow);
        assert!(result.stats.sort_candidates_retained < result.stats.matched_rows);
        assert_eq!(
            materialize_collection_result(result),
            baseline(&data, &filters, &sort, pagination)
        );
    }

    #[test]
    fn deep_sorted_window_falls_back_to_full_materialization() {
        let data = (1..=2_000)
            .map(|id| json!({"id": id, "rank": id % 13}))
            .collect::<Vec<_>>();
        let sort = vec![SortColumn { field_path: "rank".to_string(), descending: false }];
        let pagination = Pagination { page: 20, per_page: 100 };

        let result = execute_collection_query(&data, &[], &sort, Some(pagination), None);
        assert_eq!(result.plan, CollectionExecutionPlan::FullMaterialization);
        assert_eq!(
            materialize_collection_result(result),
            baseline(&data, &[], &sort, pagination)
        );
    }
}
