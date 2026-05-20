use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::schema::{
    ColumnSchema, ColumnType, DeclaredSchema, DeclaredTableSchema, ForeignKey, TableKind,
    is_valid_identifier,
};

pub fn parse_xsd_schema(input: &str) -> Result<DeclaredSchema, String> {
    let document =
        roxmltree::Document::parse(input).map_err(|err| format!("invalid schema xsd: {err}"))?;
    let schema = document
        .descendants()
        .find(|node| node.is_element() && is_xsd_node(*node, "schema"))
        .ok_or_else(|| "invalid schema xsd: missing schema element".to_string())?;

    let type_registry = collect_xsd_type_registry(schema);
    let mut tables = BTreeMap::new();
    for element in xsd_child_elements(schema, "element") {
        let nested_tables = repeated_complex_child_elements(element, &type_registry);
        if nested_tables.is_empty() {
            let (table_name, table) = parse_xsd_table_element(element, &type_registry)?;
            tables.insert(table_name, table);
        } else {
            for nested in nested_tables {
                let (table_name, table) = parse_xsd_table_element(nested, &type_registry)?;
                tables.insert(table_name, table);
            }
        }
    }

    let key_targets = apply_xsd_keys(&document, &mut tables);
    apply_xsd_unique_constraints(&document, &mut tables);
    apply_xsd_keyrefs(&document, &mut tables, &key_targets);

    Ok(DeclaredSchema { tables })
}

struct XsdTypeRegistry<'a, 'input> {
    complex_types: HashMap<String, roxmltree::Node<'a, 'input>>,
    simple_types: HashMap<String, roxmltree::Node<'a, 'input>>,
}
fn parse_xsd_table_element(
    table_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
) -> Result<(String, DeclaredTableSchema), String> {
    let table_name = table_element
        .attribute("name")
        .ok_or_else(|| "xsd table element is missing a name".to_string())?;
    if !is_valid_identifier(table_name) {
        return Err(format!("xsd table element has invalid name '{table_name}'"));
    }

    let mut table = DeclaredTableSchema::default();
    if let Some(complex_type) = xsd_complex_type_for_element(table_element, type_registry) {
        for column_element in xsd_compositor_child_elements(complex_type) {
            let Some(column_name) = column_element.attribute("name") else {
                continue;
            };
            if !is_valid_identifier(column_name) {
                return Err(format!(
                    "xsd table '{table_name}' has invalid column name '{column_name}'"
                ));
            }
            table
                .columns
                .insert(column_name.to_string(), parse_xsd_column(column_element, type_registry));
        }
    }

    Ok((table_name.to_string(), table))
}

fn parse_xsd_column(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
) -> ColumnSchema {
    let mut column = ColumnSchema::new(
        xsd_column_type(column_element, type_registry),
        xsd_column_is_nullable(column_element),
    );
    column.enum_values = parse_xsd_enum_values(column_element, type_registry);
    column.min = parse_xsd_bound(column_element, type_registry, "minInclusive")
        .or_else(|| parse_xsd_bound(column_element, type_registry, "minExclusive"));
    column.max = parse_xsd_bound(column_element, type_registry, "maxInclusive")
        .or_else(|| parse_xsd_bound(column_element, type_registry, "maxExclusive"));
    column.min_length = parse_xsd_usize_facet(column_element, type_registry, "minLength");
    column.max_length = parse_xsd_usize_facet(column_element, type_registry, "maxLength");
    column.pattern = parse_xsd_string_facet(column_element, type_registry, "pattern");
    column
}

fn xsd_column_is_nullable(column_element: roxmltree::Node<'_, '_>) -> bool {
    matches!(column_element.attribute("minOccurs"), Some("0"))
        || matches!(column_element.attribute("nillable"), Some("true") | Some("1"))
}

fn xsd_column_type(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
) -> ColumnType {
    if has_repeating_occurs(column_element)
        || xsd_complex_type_for_element(column_element, type_registry).is_some()
    {
        return ColumnType::Json;
    }

    if let Some(restriction) = xsd_restriction_for_element(column_element, type_registry)
        && let Some(base) = restriction.attribute("base")
    {
        return ColumnType::from_xsd_type(base);
    }

    column_element.attribute("type").map(ColumnType::from_xsd_type).unwrap_or(ColumnType::String)
}

fn parse_xsd_enum_values(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
) -> Option<Vec<String>> {
    let restriction = xsd_restriction_for_element(column_element, type_registry)?;
    let values = xsd_child_elements(restriction, "enumeration")
        .filter_map(|node| node.attribute("value").map(str::to_string))
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn parse_xsd_bound(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
    facet: &str,
) -> Option<Value> {
    let value = xsd_restriction_facet_value(column_element, type_registry, facet)?;
    Some(
        value
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| value.parse::<f64>().map(Value::from))
            .unwrap_or_else(|_| Value::String(value.to_string())),
    )
}

fn parse_xsd_usize_facet(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
    facet: &str,
) -> Option<usize> {
    xsd_restriction_facet_value(column_element, type_registry, facet)?.parse().ok()
}

fn parse_xsd_string_facet(
    column_element: roxmltree::Node<'_, '_>,
    type_registry: &XsdTypeRegistry<'_, '_>,
    facet: &str,
) -> Option<String> {
    xsd_restriction_facet_value(column_element, type_registry, facet).map(str::to_string)
}

fn xsd_restriction_facet_value<'a, 'input>(
    column_element: roxmltree::Node<'a, 'input>,
    type_registry: &XsdTypeRegistry<'a, 'input>,
    facet: &str,
) -> Option<&'a str> {
    let restriction = xsd_restriction_for_element(column_element, type_registry)?;
    xsd_child_element(restriction, facet)?.attribute("value")
}

fn xsd_restriction_for_element<'a, 'input>(
    element: roxmltree::Node<'a, 'input>,
    type_registry: &XsdTypeRegistry<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
    xsd_simple_type_for_element(element, type_registry)
        .and_then(|simple_type| xsd_child_element(simple_type, "restriction"))
}

fn xsd_simple_type_for_element<'a, 'input>(
    element: roxmltree::Node<'a, 'input>,
    type_registry: &XsdTypeRegistry<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
    xsd_child_element(element, "simpleType").or_else(|| {
        element
            .attribute("type")
            .map(normalize_qname)
            .and_then(|type_name| type_registry.simple_types.get(&type_name).copied())
    })
}

fn xsd_complex_type_for_element<'a, 'input>(
    element: roxmltree::Node<'a, 'input>,
    type_registry: &XsdTypeRegistry<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
    xsd_child_element(element, "complexType").or_else(|| {
        element
            .attribute("type")
            .map(normalize_qname)
            .and_then(|type_name| type_registry.complex_types.get(&type_name).copied())
    })
}

fn apply_xsd_keys(
    document: &roxmltree::Document<'_>,
    tables: &mut BTreeMap<String, DeclaredTableSchema>,
) -> HashMap<String, ForeignKey> {
    let mut targets = HashMap::new();
    for key in document.descendants().filter(|node| node.is_element() && is_xsd_node(*node, "key"))
    {
        let Some(table_name) = xsd_constraint_table(key, tables) else {
            continue;
        };
        let Some(column_name) = xsd_constraint_field(key) else {
            continue;
        };
        let Some(table) = tables.get_mut(&table_name) else {
            continue;
        };
        if !table.columns.contains_key(&column_name) {
            continue;
        }
        table.primary_key = Some(column_name.clone());
        table.kind = Some(TableKind::Object);

        if let Some(key_name) = key.attribute("name").map(normalize_qname) {
            targets.insert(
                key_name,
                ForeignKey { target_table: table_name, target_column: column_name },
            );
        }
    }
    targets
}

fn apply_xsd_unique_constraints(
    document: &roxmltree::Document<'_>,
    tables: &mut BTreeMap<String, DeclaredTableSchema>,
) {
    for unique in
        document.descendants().filter(|node| node.is_element() && is_xsd_node(*node, "unique"))
    {
        let Some(table_name) = xsd_constraint_table(unique, tables) else {
            continue;
        };
        let Some(column_name) = xsd_constraint_field(unique) else {
            continue;
        };
        let Some(table) = tables.get_mut(&table_name) else {
            continue;
        };
        if table.columns.contains_key(&column_name) {
            table.unique.push(vec![column_name]);
        }
    }
}

fn apply_xsd_keyrefs(
    document: &roxmltree::Document<'_>,
    tables: &mut BTreeMap<String, DeclaredTableSchema>,
    key_targets: &HashMap<String, ForeignKey>,
) {
    for keyref in
        document.descendants().filter(|node| node.is_element() && is_xsd_node(*node, "keyref"))
    {
        let Some(source_table) = xsd_constraint_table(keyref, tables) else {
            continue;
        };
        let Some(source_column) = xsd_constraint_field(keyref) else {
            continue;
        };
        let Some(target) =
            keyref.attribute("refer").map(normalize_qname).and_then(|name| key_targets.get(&name))
        else {
            continue;
        };
        let Some(table) = tables.get_mut(&source_table) else {
            continue;
        };
        if table.columns.contains_key(&source_column) {
            table.foreign_keys.insert(source_column, target.clone());
        }
    }
}

fn xsd_constraint_table(
    constraint: roxmltree::Node<'_, '_>,
    tables: &BTreeMap<String, DeclaredTableSchema>,
) -> Option<String> {
    let selector_name = xsd_child_element(constraint, "selector")
        .and_then(|selector| selector.attribute("xpath"))
        .and_then(normalize_xsd_xpath_name);
    if let Some(table_name) = selector_name
        && tables.contains_key(&table_name)
    {
        return Some(table_name);
    }

    constraint
        .ancestors()
        .filter(|node| node.is_element() && is_xsd_node(*node, "element"))
        .filter_map(|node| node.attribute("name").map(str::to_string))
        .find(|name| tables.contains_key(name))
}

fn xsd_constraint_field(constraint: roxmltree::Node<'_, '_>) -> Option<String> {
    xsd_child_element(constraint, "field")
        .and_then(|field| field.attribute("xpath"))
        .and_then(normalize_xsd_xpath_name)
}

fn collect_xsd_type_registry<'a, 'input>(
    schema: roxmltree::Node<'a, 'input>,
) -> XsdTypeRegistry<'a, 'input> {
    XsdTypeRegistry {
        complex_types: xsd_child_elements(schema, "complexType")
            .filter_map(|node| node.attribute("name").map(|name| (normalize_qname(name), node)))
            .collect(),
        simple_types: xsd_child_elements(schema, "simpleType")
            .filter_map(|node| node.attribute("name").map(|name| (normalize_qname(name), node)))
            .collect(),
    }
}

fn repeated_complex_child_elements<'a, 'input>(
    element: roxmltree::Node<'a, 'input>,
    type_registry: &XsdTypeRegistry<'a, 'input>,
) -> Vec<roxmltree::Node<'a, 'input>> {
    let Some(complex_type) = xsd_complex_type_for_element(element, type_registry) else {
        return Vec::new();
    };
    xsd_compositor_child_elements(complex_type)
        .filter(|child| {
            has_repeating_occurs(*child)
                && xsd_complex_type_for_element(*child, type_registry).is_some()
        })
        .collect()
}

fn xsd_compositor_child_elements<'a, 'input>(
    complex_type: roxmltree::Node<'a, 'input>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    xsd_child_elements(complex_type, "sequence")
        .chain(xsd_child_elements(complex_type, "all"))
        .chain(xsd_child_elements(complex_type, "choice"))
        .flat_map(|node| xsd_child_elements(node, "element"))
}

fn xsd_child_element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    xsd_child_elements(node, local_name).next()
}

fn xsd_child_elements<'a, 'input, 'name>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &'name str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> + 'name
where
    'a: 'name,
    'input: 'name,
{
    node.children().filter(move |child| child.is_element() && is_xsd_node(*child, local_name))
}

fn is_xsd_node(node: roxmltree::Node<'_, '_>, local_name: &str) -> bool {
    node.tag_name().name() == local_name
}

fn has_repeating_occurs(element: roxmltree::Node<'_, '_>) -> bool {
    element.attribute("maxOccurs").is_some_and(|value| {
        value == "unbounded" || value.parse::<usize>().is_ok_and(|count| count > 1)
    })
}

fn normalize_qname(name: &str) -> String {
    name.rsplit_once(':').map(|(_, local)| local).unwrap_or(name).to_string()
}

fn normalize_xsd_xpath_name(xpath: &str) -> Option<String> {
    let cleaned =
        xpath.trim().trim_start_matches("./").trim_start_matches(".//").trim_start_matches('/');
    if cleaned == "." || cleaned.is_empty() {
        return None;
    }
    let last_segment = cleaned.rsplit('/').next()?.trim_start_matches('@');
    let local = normalize_qname(last_segment);
    is_valid_identifier(&local).then_some(local)
}
