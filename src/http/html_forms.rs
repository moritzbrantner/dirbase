use std::collections::{BTreeMap, BTreeSet};

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::Html,
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    http::html::{encode_path_segment, escape_html},
    schema::{ColumnType, TableSchema, primary_key_name},
    storage::{load_resource, validate_resource_data},
};

pub async fn get_resource_editor(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
) -> Html<String> {
    Html(render_patch_editor_html(
        &format!("/{}", encode_path_segment(&resource)),
        state.config.readonly || state.config.clone_proxy.is_some(),
    ))
}

pub async fn get_item_editor(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Html<String> {
    Html(render_patch_editor_html(
        &format!("/{}/{}", encode_path_segment(&resource), encode_path_segment(&id)),
        state.config.readonly || state.config.clone_proxy.is_some(),
    ))
}

pub async fn get_create_item_form(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
) -> Result<Html<String>, AppError> {
    let _guard = state.read_lock_for_resource(&resource).await;
    let data = load_resource(&state, &resource).await?;
    validate_resource_data(&state, &resource, data.as_ref())?;
    let items = data.as_array().ok_or_else(|| {
        AppError::new(StatusCode::BAD_REQUEST, "Create forms require a JSON array resource")
    })?;
    let table = state.schema_table(&resource);
    let fields = create_form_fields(items, table.as_ref());
    Ok(Html(render_create_item_form_html(
        &resource,
        &format!("/{}", encode_path_segment(&resource)),
        &fields,
        state.config.readonly || state.config.clone_proxy.is_some(),
    )))
}

fn render_patch_editor_html(target_path: &str, readonly: bool) -> String {
    let target_path_json =
        serde_json::to_string(target_path).expect("serializing an editor path cannot fail");
    let readonly_json =
        serde_json::to_string(&readonly).expect("serializing readonly flag cannot fail");
    let escaped_target_path = escape_html(target_path);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Edit {escaped_target_path}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f8fafc;
      color: #1f2937;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    header {{
      border-bottom: 1px solid #d6d3d1;
      background: #ffffff;
      padding: 16px clamp(16px, 4vw, 40px);
    }}
    main {{
      flex: 1;
      display: grid;
      grid-template-rows: auto minmax(320px, 1fr) auto;
      gap: 12px;
      padding: 16px clamp(16px, 4vw, 40px) 24px;
    }}
    h1 {{
      font-size: 20px;
      line-height: 1.25;
      margin: 0 0 6px;
      overflow-wrap: anywhere;
    }}
    .route {{
      display: inline-block;
      max-width: 100%;
      overflow-wrap: anywhere;
      border: 1px solid #d6d3d1;
      background: #f5f5f4;
      border-radius: 6px;
      padding: 4px 8px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
    }}
    .toolbar {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
    }}
    .status {{
      min-height: 22px;
      font-size: 14px;
      color: #57534e;
    }}
    .status.error {{
      color: #b91c1c;
    }}
    .status.ok {{
      color: #047857;
    }}
    textarea {{
      width: 100%;
      min-height: 100%;
      box-sizing: border-box;
      resize: vertical;
      border: 1px solid #a8a29e;
      border-radius: 8px;
      background: #ffffff;
      padding: 14px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      line-height: 1.55;
      color: #1c1917;
      tab-size: 2;
    }}
    button {{
      border: 1px solid #78716c;
      border-radius: 6px;
      background: #292524;
      color: #ffffff;
      font: inherit;
      font-weight: 600;
      padding: 8px 12px;
      cursor: pointer;
    }}
    button.secondary {{
      background: #ffffff;
      color: #292524;
    }}
    button:disabled {{
      cursor: not-allowed;
      opacity: 0.55;
    }}
    .actions {{
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      flex-wrap: wrap;
    }}
  </style>
</head>
<body>
  <header>
    <h1>Edit JSON resource</h1>
    <code class="route" id="target-path">{escaped_target_path}</code>
  </header>
  <main>
    <div class="toolbar">
      <div class="status" id="status" role="status">Loading resource...</div>
      <button type="button" class="secondary" id="reload-button">Reload</button>
    </div>
    <textarea id="editor" spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off"></textarea>
    <div class="actions">
      <button type="button" class="secondary" id="format-button">Format JSON</button>
      <button type="button" id="save-button">Save patch</button>
    </div>
  </main>
  <script>
    const targetPath = {target_path_json};
    const readonly = {readonly_json};
    const editor = document.getElementById('editor');
    const statusNode = document.getElementById('status');
    const saveButton = document.getElementById('save-button');
    const reloadButton = document.getElementById('reload-button');
    const formatButton = document.getElementById('format-button');
    let originalValue = null;

    function setStatus(message, kind = '') {{
      statusNode.textContent = message;
      statusNode.className = kind ? `status ${{kind}}` : 'status';
    }}

    function formatJson(value) {{
      return JSON.stringify(value, null, 2) + '\n';
    }}

    function isPlainObject(value) {{
      return value !== null && typeof value === 'object' && !Array.isArray(value);
    }}

    function sameJson(left, right) {{
      return JSON.stringify(left) === JSON.stringify(right);
    }}

    function buildPatch(original, next) {{
      if (!isPlainObject(original) || !isPlainObject(next)) {{
        throw new Error('PATCH editing requires the loaded resource to be a JSON object.');
      }}
      const removedKeys = Object.keys(original).filter((key) => !Object.prototype.hasOwnProperty.call(next, key));
      if (removedKeys.length > 0) {{
        throw new Error(`PATCH cannot remove keys: ${{removedKeys.join(', ')}}. Set a value to null or use PUT from the API.`);
      }}
      const patch = {{}};
      for (const [key, value] of Object.entries(next)) {{
        if (!sameJson(original[key], value)) {{
          patch[key] = value;
        }}
      }}
      return patch;
    }}

    async function loadResource() {{
      saveButton.disabled = true;
      reloadButton.disabled = true;
      setStatus('Loading resource...');
      try {{
        const response = await fetch(targetPath, {{ headers: {{ Accept: 'application/json' }} }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `GET failed: ${{response.status}} ${{response.statusText}}`);
        }}
        originalValue = JSON.parse(text);
        editor.value = formatJson(originalValue);
        setStatus(readonly ? 'Read-only mode: editing is disabled.' : 'Loaded. Edit JSON and save a PATCH request.', readonly ? 'error' : 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to load resource.', 'error');
      }} finally {{
        saveButton.disabled = readonly;
        reloadButton.disabled = false;
      }}
    }}

    async function savePatch() {{
      if (readonly) {{
        return;
      }}
      saveButton.disabled = true;
      reloadButton.disabled = true;
      try {{
        const nextValue = JSON.parse(editor.value);
        const patch = buildPatch(originalValue, nextValue);
        const changedKeys = Object.keys(patch);
        if (changedKeys.length === 0) {{
          setStatus('No top-level changes to patch.', 'error');
          return;
        }}
        setStatus(`Saving PATCH with ${{changedKeys.length}} changed key(s)...`);
        const response = await fetch(targetPath, {{
          method: 'PATCH',
          headers: {{
            Accept: 'application/json',
            'Content-Type': 'application/json'
          }},
          body: JSON.stringify(patch)
        }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `PATCH failed: ${{response.status}} ${{response.statusText}}`);
        }}
        originalValue = JSON.parse(text);
        editor.value = formatJson(originalValue);
        setStatus(`Saved PATCH for: ${{changedKeys.join(', ')}}`, 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to save patch.', 'error');
      }} finally {{
        saveButton.disabled = readonly;
        reloadButton.disabled = false;
      }}
    }}

    function formatDraft() {{
      try {{
        editor.value = formatJson(JSON.parse(editor.value));
        setStatus('Formatted JSON.', 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Invalid JSON.', 'error');
      }}
    }}

    reloadButton.addEventListener('click', () => void loadResource());
    saveButton.addEventListener('click', () => void savePatch());
    formatButton.addEventListener('click', formatDraft);
    void loadResource();
  </script>
</body>
</html>
"#
    )
}

#[derive(Serialize)]
struct CreateFormField {
    name: String,
    field_type: &'static str,
    nullable: bool,
    primary_key: bool,
    sample: Option<Value>,
}

fn create_form_fields(items: &[Value], table: Option<&TableSchema>) -> Vec<CreateFormField> {
    let primary_key = primary_key_name(table);
    let sample_object = items.iter().find_map(Value::as_object);
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(table) = table {
        for (name, column) in &table.columns {
            seen.insert(name.clone());
            fields.push(CreateFormField {
                name: name.clone(),
                field_type: create_form_column_type(&column.column_type),
                nullable: column.nullable,
                primary_key: name == primary_key,
                sample: sample_object.and_then(|object| object.get(name)).cloned(),
            });
        }
    }

    if let Some(object) = sample_object {
        let mut sampled = BTreeMap::new();
        for (name, value) in object {
            sampled.insert(name.clone(), value.clone());
        }
        for (name, value) in sampled {
            if !seen.insert(name.clone()) {
                continue;
            }
            fields.push(CreateFormField {
                name: name.clone(),
                field_type: infer_create_form_field_type(&value),
                nullable: true,
                primary_key: name == primary_key,
                sample: Some(value),
            });
        }
    }

    if fields.is_empty() {
        fields.push(CreateFormField {
            name: "name".to_string(),
            field_type: "string",
            nullable: false,
            primary_key: false,
            sample: None,
        });
    }

    fields.sort_by(|left, right| {
        right.primary_key.cmp(&left.primary_key).then_with(|| left.name.cmp(&right.name))
    });
    fields
}

fn create_form_column_type(column_type: &ColumnType) -> &'static str {
    column_type.label()
}

fn infer_create_form_field_type(value: &Value) -> &'static str {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "float",
        Value::Bool(_) => "boolean",
        Value::Array(_) | Value::Object(_) => "json",
        _ => "string",
    }
}

fn render_create_item_form_html(
    resource: &str,
    target_path: &str,
    fields: &[CreateFormField],
    readonly: bool,
) -> String {
    let target_path_json =
        serde_json::to_string(target_path).expect("serializing a create path cannot fail");
    let fields_json =
        serde_json::to_string(fields).expect("serializing create form fields cannot fail");
    let readonly_json =
        serde_json::to_string(&readonly).expect("serializing readonly flag cannot fail");
    let escaped_target_path = escape_html(target_path);
    let escaped_resource = escape_html(resource);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Create {escaped_target_path}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f8fafc;
      color: #1f2937;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    header {{
      border-bottom: 1px solid #d6d3d1;
      background: #ffffff;
      padding: 16px clamp(16px, 4vw, 40px);
    }}
    main {{
      flex: 1;
      display: grid;
      gap: 16px;
      padding: 16px clamp(16px, 4vw, 40px) 24px;
      max-width: 920px;
      width: 100%;
      box-sizing: border-box;
    }}
    h1 {{
      font-size: 20px;
      line-height: 1.25;
      margin: 0 0 6px;
      overflow-wrap: anywhere;
    }}
    .route, .result {{
      display: block;
      max-width: 100%;
      overflow-wrap: anywhere;
      border: 1px solid #d6d3d1;
      background: #f5f5f4;
      border-radius: 6px;
      padding: 8px 10px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      white-space: pre-wrap;
    }}
    .status {{
      min-height: 22px;
      font-size: 14px;
      color: #57534e;
    }}
    .status.error {{
      color: #b91c1c;
    }}
    .status.ok {{
      color: #047857;
    }}
    form {{
      display: grid;
      gap: 14px;
    }}
    .field-row {{
      display: grid;
      gap: 8px;
      border: 1px solid #e7e5e4;
      border-radius: 8px;
      background: #ffffff;
      padding: 12px;
    }}
    .field-head {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      flex-wrap: wrap;
    }}
    label {{
      font-weight: 650;
      color: #292524;
    }}
    .badges {{
      display: flex;
      gap: 6px;
      flex-wrap: wrap;
    }}
    .badge {{
      border: 1px solid #d6d3d1;
      border-radius: 999px;
      padding: 2px 7px;
      font-size: 12px;
      color: #57534e;
      background: #fafaf9;
    }}
    .control-grid {{
      display: grid;
      grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
      gap: 8px;
    }}
    @media (max-width: 640px) {{
      .control-grid {{
        grid-template-columns: 1fr;
      }}
    }}
    input, select, textarea {{
      width: 100%;
      box-sizing: border-box;
      border: 1px solid #a8a29e;
      border-radius: 6px;
      background: #ffffff;
      padding: 8px 10px;
      font: inherit;
      color: #1c1917;
    }}
    textarea {{
      min-height: 96px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      line-height: 1.5;
      resize: vertical;
    }}
    button {{
      border: 1px solid #78716c;
      border-radius: 6px;
      background: #292524;
      color: #ffffff;
      font: inherit;
      font-weight: 600;
      padding: 8px 12px;
      cursor: pointer;
    }}
    button.secondary {{
      background: #ffffff;
      color: #292524;
    }}
    button:disabled, input:disabled, select:disabled, textarea:disabled {{
      cursor: not-allowed;
      opacity: 0.55;
    }}
    .actions {{
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      flex-wrap: wrap;
    }}
    .custom-name {{
      display: none;
    }}
    .field-row.is-custom .custom-name {{
      display: block;
    }}
  </style>
</head>
<body>
  <header>
    <h1>Create item in {escaped_resource}</h1>
    <code class="route" id="target-path">POST {escaped_target_path}</code>
  </header>
  <main>
    <div class="status" id="status" role="status"></div>
    <form id="create-form">
      <div id="fields"></div>
      <div class="actions">
        <button type="button" class="secondary" id="add-field-button">Add field</button>
        <button type="reset" class="secondary" id="reset-button">Reset</button>
        <button type="submit" id="submit-button">Create item</button>
      </div>
    </form>
    <pre class="result" id="result" hidden></pre>
  </main>
  <script>
    const targetPath = {target_path_json};
    const readonly = {readonly_json};
    const fields = {fields_json};
    const fieldsNode = document.getElementById('fields');
    const form = document.getElementById('create-form');
    const statusNode = document.getElementById('status');
    const resultNode = document.getElementById('result');
    const submitButton = document.getElementById('submit-button');
    const addFieldButton = document.getElementById('add-field-button');
    let customFieldIndex = 0;

    function setStatus(message, kind = '') {{
      statusNode.textContent = message;
      statusNode.className = kind ? `status ${{kind}}` : 'status';
    }}

    function formatSample(value) {{
      if (value === null || value === undefined) {{
        return '';
      }}
      if (typeof value === 'object') {{
        return JSON.stringify(value);
      }}
      return String(value);
    }}

    function createValueControl(field) {{
      if (field.field_type === 'boolean') {{
        const select = document.createElement('select');
        select.name = 'value';
        select.append(new Option(field.nullable || field.primary_key ? 'Blank' : 'False', ''));
        select.append(new Option('True', 'true'));
        select.append(new Option('False', 'false'));
        select.required = !field.nullable && !field.primary_key;
        return select;
      }}
      if (field.field_type === 'json') {{
        const textarea = document.createElement('textarea');
        textarea.name = 'value';
        textarea.placeholder = formatSample(field.sample) || '{{}}';
        textarea.required = !field.nullable && !field.primary_key;
        return textarea;
      }}
      const input = document.createElement('input');
      input.name = 'value';
      input.type = field.field_type === 'integer' || field.field_type === 'float' ? 'number' : 'text';
      if (field.field_type === 'integer') {{
        input.step = '1';
      }} else if (field.field_type === 'float') {{
        input.step = 'any';
      }}
      input.placeholder = field.primary_key ? 'auto-generated if blank' : formatSample(field.sample);
      input.required = !field.nullable && !field.primary_key;
      return input;
    }}

    function renderField(field, custom = false) {{
      const row = document.createElement('section');
      row.className = custom ? 'field-row is-custom' : 'field-row';
      row.dataset.fieldType = field.field_type;
      if (!custom) {{
        row.dataset.fieldName = field.name;
      }}
      row.dataset.nullable = String(field.nullable);
      row.dataset.primaryKey = String(field.primary_key);

      const head = document.createElement('div');
      head.className = 'field-head';
      const label = document.createElement('label');
      label.textContent = field.name;
      const badges = document.createElement('div');
      badges.className = 'badges';
      for (const badgeText of [
        field.field_type,
        field.primary_key ? 'primary key' : null,
        field.nullable || field.primary_key ? 'optional' : 'required'
      ].filter(Boolean)) {{
        const badge = document.createElement('span');
        badge.className = 'badge';
        badge.textContent = badgeText;
        badges.append(badge);
      }}
      head.append(label, badges);

      const grid = document.createElement('div');
      grid.className = 'control-grid';
      const typeSelect = document.createElement('select');
      typeSelect.name = 'type';
      for (const type of ['string', 'integer', 'float', 'boolean', 'json']) {{
        typeSelect.append(new Option(type, type));
      }}
      typeSelect.value = field.field_type;
      typeSelect.addEventListener('change', () => {{
        row.dataset.fieldType = typeSelect.value;
        valueSlot.replaceChildren(createValueControl({{ ...field, field_type: typeSelect.value }}));
      }});

      const valueSlot = document.createElement('div');
      valueSlot.append(createValueControl(field));
      grid.append(typeSelect, valueSlot);

      const customName = document.createElement('input');
      customName.className = 'custom-name';
      customName.name = 'name';
      customName.placeholder = 'field_name';
      customName.required = custom;

      row.append(head, customName, grid);
      if (readonly) {{
        for (const control of row.querySelectorAll('input, select, textarea')) {{
          control.disabled = true;
        }}
      }}
      return row;
    }}

    function renderFields() {{
      fieldsNode.replaceChildren(...fields.map((field) => renderField(field)));
      resultNode.hidden = true;
      resultNode.textContent = '';
      setStatus(readonly ? 'Read-only mode: item creation is disabled.' : 'Enter values and submit a POST request.', readonly ? 'error' : 'ok');
      submitButton.disabled = readonly;
      addFieldButton.disabled = readonly;
    }}

    function parseFieldValue(type, rawValue, fieldName) {{
      if (type === 'integer') {{
        const number = Number(rawValue);
        if (!Number.isInteger(number)) {{
          throw new Error(`${{fieldName}} must be an integer.`);
        }}
        return number;
      }}
      if (type === 'float') {{
        const number = Number(rawValue);
        if (!Number.isFinite(number)) {{
          throw new Error(`${{fieldName}} must be a number.`);
        }}
        return number;
      }}
      if (type === 'boolean') {{
        if (rawValue !== 'true' && rawValue !== 'false') {{
          throw new Error(`${{fieldName}} must be true or false.`);
        }}
        return rawValue === 'true';
      }}
      if (type === 'json') {{
        return JSON.parse(rawValue);
      }}
      return rawValue;
    }}

    function buildPayload() {{
      const payload = {{}};
      for (const row of fieldsNode.querySelectorAll('.field-row')) {{
        const fieldName = row.dataset.fieldName || row.querySelector('input[name="name"]').value.trim();
        const type = row.querySelector('select[name="type"]').value;
        const valueControl = row.querySelector('[name="value"]');
        const rawValue = valueControl.value;
        const optional = row.dataset.nullable === 'true' || row.dataset.primaryKey === 'true';
        if (!fieldName) {{
          throw new Error('Custom fields need a name.');
        }}
        if (rawValue === '' && optional) {{
          continue;
        }}
        payload[fieldName] = parseFieldValue(type, rawValue, fieldName);
      }}
      return payload;
    }}

    form.addEventListener('submit', async (event) => {{
      event.preventDefault();
      if (readonly) {{
        return;
      }}
      submitButton.disabled = true;
      resultNode.hidden = true;
      try {{
        const payload = buildPayload();
        setStatus('Creating item...');
        const response = await fetch(targetPath, {{
          method: 'POST',
          headers: {{
            Accept: 'application/json',
            'Content-Type': 'application/json'
          }},
          body: JSON.stringify(payload)
        }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `POST failed: ${{response.status}} ${{response.statusText}}`);
        }}
        const created = JSON.parse(text);
        resultNode.hidden = false;
        resultNode.textContent = JSON.stringify(created, null, 2);
        setStatus('Created item.', 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to create item.', 'error');
      }} finally {{
        submitButton.disabled = readonly;
      }}
    }});

    form.addEventListener('reset', () => {{
      setTimeout(renderFields, 0);
    }});

    addFieldButton.addEventListener('click', () => {{
      customFieldIndex += 1;
      fieldsNode.append(renderField({{
        name: `Custom field ${{customFieldIndex}}`,
        field_type: 'string',
        nullable: true,
        primary_key: false,
        sample: null
      }}, true));
    }});

    renderFields();
  </script>
</body>
</html>
"#
    )
}
