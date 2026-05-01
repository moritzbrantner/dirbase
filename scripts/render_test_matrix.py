#!/usr/bin/env python3
"""Validate and render the semantic test coverage matrix."""

from __future__ import annotations

import argparse
import datetime as dt
import http.server
import json
import os
import re
import socketserver
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = ROOT / "docs" / "testing" / "test-matrix.json"
OUTPUT_DIR = ROOT / "target" / "test-matrix"

ALLOWED_RISKS = {"critical", "high", "medium", "low"}
ALLOWED_PATH_STATUSES = {"covered", "partial", "missing", "not_applicable"}
ALLOWED_TEST_TYPES = {"unit", "integration", "e2e", "coverage_metric", "manual_reference"}
REQUIRED_PATH_KINDS = ("happy", "exceptional", "concurrency")

SOURCE_GLOBS = (
    "src/**/*.rs",
    "ui/src/**/*.ts",
    "ui/src/**/*.tsx",
    "js/src/**/*.ts",
)
TEST_GLOBS = (
    "src/**/*.rs",
    "tests/**/*.rs",
    "ui/src/**/*.test.ts",
    "ui/src/**/*.test.tsx",
    "ui/e2e/**/*.spec.ts",
    "ui/e2e/**/*.spec.cjs",
    "js/src/**/*.test.ts",
)


class MatrixIssue:
    def __init__(self, level: str, message: str) -> None:
        self.level = level
        self.message = message

    def __str__(self) -> str:
        return f"{self.level.upper()}: {self.message}"


def relpath(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def load_matrix() -> dict[str, Any]:
    with MATRIX_PATH.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def iter_files(patterns: tuple[str, ...]) -> list[Path]:
    files: list[Path] = []
    for pattern in patterns:
        files.extend(ROOT.glob(pattern))
    return sorted({path for path in files if path.is_file()})


def discover_tests() -> dict[str, set[str]]:
    tests: dict[str, set[str]] = defaultdict(set)
    rust_pending_test = False
    rust_fn = re.compile(r"^\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
    js_named_test = re.compile(r"\b(?:describe|it|test)\(\s*(['\"])(.*?)\1")

    for path in iter_files(TEST_GLOBS):
        relative = relpath(path)
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue

        if path.suffix == ".rs":
            rust_pending_test = False
            for line in lines:
                if "#[test]" in line or "#[tokio::test]" in line:
                    rust_pending_test = True
                    continue
                match = rust_fn.match(line)
                if match and rust_pending_test:
                    tests[match.group(1)].add(relative)
                    rust_pending_test = False
            continue

        for line in lines:
            for match in js_named_test.finditer(line):
                tests[match.group(2)].add(relative)

    return tests


def discover_public_symbols() -> dict[str, set[str]]:
    symbols: dict[str, set[str]] = defaultdict(set)
    rust_patterns = (
        re.compile(r"^\s*pub\s+(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\("),
        re.compile(r"^\s*pub\s+(?:struct|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)\b"),
    )
    ts_patterns = (
        re.compile(r"^\s*export\s+(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*\("),
        re.compile(r"^\s*export\s+const\s+([A-Za-z_$][A-Za-z0-9_$]*)\b"),
        re.compile(r"^\s*export\s+(?:interface|type|class)\s+([A-Za-z_$][A-Za-z0-9_$]*)\b"),
        re.compile(r"^\s*export\s+default\s+function\s+([A-Za-z_$][A-Za-z0-9_$]*)?\s*\("),
    )

    for path in iter_files(SOURCE_GLOBS):
        relative = relpath(path)
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue

        patterns = rust_patterns if path.suffix == ".rs" else ts_patterns
        for line in lines:
            for pattern in patterns:
                match = pattern.match(line)
                if match:
                    name = match.group(1) or "default"
                    symbols[relative].add(name)
                    break

    return symbols


def validate_matrix(matrix: dict[str, Any], tests: dict[str, set[str]], symbols: dict[str, set[str]]) -> list[MatrixIssue]:
    issues: list[MatrixIssue] = []

    if matrix.get("version") != 1:
        issues.append(MatrixIssue("error", "matrix version must be 1"))

    areas = matrix.get("areas")
    entries = matrix.get("entries")
    if not isinstance(areas, list):
        issues.append(MatrixIssue("error", "areas must be a list"))
        areas = []
    if not isinstance(entries, list):
        issues.append(MatrixIssue("error", "entries must be a list"))
        entries = []

    area_ids: set[str] = set()
    entry_ids: set[str] = set()

    for area in areas:
        area_id = area.get("id")
        if not area_id:
            issues.append(MatrixIssue("error", "area is missing id"))
            continue
        if area_id in area_ids:
            issues.append(MatrixIssue("error", f"duplicate area id: {area_id}"))
        area_ids.add(area_id)
        if area.get("risk") not in ALLOWED_RISKS:
            issues.append(MatrixIssue("error", f"area {area_id} has invalid risk: {area.get('risk')}"))
        if not isinstance(area.get("owner_paths"), list) or not area["owner_paths"]:
            issues.append(MatrixIssue("error", f"area {area_id} must list owner_paths"))

    covered_symbols_by_path: dict[str, set[str]] = defaultdict(set)
    covered_symbol_names: set[str] = set()

    for entry in entries:
        entry_id = entry.get("id")
        if not entry_id:
            issues.append(MatrixIssue("error", "entry is missing id"))
            continue
        if entry_id in entry_ids:
            issues.append(MatrixIssue("error", f"duplicate entry id: {entry_id}"))
        entry_ids.add(entry_id)

        if entry.get("area") not in area_ids:
            issues.append(MatrixIssue("error", f"entry {entry_id} references unknown area: {entry.get('area')}"))
        if entry.get("risk") not in ALLOWED_RISKS:
            issues.append(MatrixIssue("error", f"entry {entry_id} has invalid risk: {entry.get('risk')}"))
        if not entry.get("language"):
            issues.append(MatrixIssue("error", f"entry {entry_id} is missing language"))
        if not entry.get("module"):
            issues.append(MatrixIssue("error", f"entry {entry_id} is missing module"))
        if not entry.get("symbol"):
            issues.append(MatrixIssue("error", f"entry {entry_id} is missing symbol"))

        module = entry.get("module")
        if isinstance(module, str):
            for symbol in entry.get("covered_symbols", []):
                covered_symbols_by_path[module].add(symbol)
                covered_symbol_names.add(symbol)
            if entry.get("symbol"):
                covered_symbols_by_path[module].add(str(entry["symbol"]))
                covered_symbol_names.add(str(entry["symbol"]))

        covered_by = entry.get("covered_by")
        if not isinstance(covered_by, list) or not covered_by:
            issues.append(MatrixIssue("error", f"entry {entry_id} must include covered_by tests"))
        else:
            for reference in covered_by:
                test_name = reference.get("test")
                test_path = reference.get("path")
                test_type = reference.get("test_type")
                if test_type not in ALLOWED_TEST_TYPES:
                    issues.append(MatrixIssue("error", f"entry {entry_id} has invalid test_type: {test_type}"))
                if not test_name:
                    issues.append(MatrixIssue("error", f"entry {entry_id} has a covered_by item without test name"))
                    continue
                if test_type == "manual_reference":
                    continue
                found_paths = tests.get(test_name, set())
                if not found_paths:
                    issues.append(MatrixIssue("error", f"entry {entry_id} references missing test: {test_name}"))
                elif test_path and test_path not in found_paths:
                    issues.append(
                        MatrixIssue(
                            "error",
                            f"entry {entry_id} references test {test_name} at {test_path}, found at {sorted(found_paths)}",
                        )
                    )

        paths = entry.get("paths")
        if not isinstance(paths, dict):
            issues.append(MatrixIssue("error", f"entry {entry_id} must include paths object"))
            continue
        for path_kind in REQUIRED_PATH_KINDS:
            path_info = paths.get(path_kind)
            if not isinstance(path_info, dict):
                issues.append(MatrixIssue("error", f"entry {entry_id} is missing paths.{path_kind}"))
                continue
            status = path_info.get("status")
            if status not in ALLOWED_PATH_STATUSES:
                issues.append(MatrixIssue("error", f"entry {entry_id} has invalid {path_kind} status: {status}"))
            if not isinstance(path_info.get("cases", []), list):
                issues.append(MatrixIssue("error", f"entry {entry_id} paths.{path_kind}.cases must be a list"))
            if "missing_cases" in path_info and not isinstance(path_info["missing_cases"], list):
                issues.append(MatrixIssue("error", f"entry {entry_id} paths.{path_kind}.missing_cases must be a list"))

        for parameter in entry.get("parameters", []):
            if not parameter.get("name"):
                issues.append(MatrixIssue("error", f"entry {entry_id} has parameter without name"))
            if not isinstance(parameter.get("covered_values", []), list):
                issues.append(MatrixIssue("error", f"entry {entry_id} parameter covered_values must be a list"))
            if not isinstance(parameter.get("missing_values", []), list):
                issues.append(MatrixIssue("error", f"entry {entry_id} parameter missing_values must be a list"))

    for area in areas:
        for entry_id in area.get("entries", []):
            if entry_id not in entry_ids:
                issues.append(MatrixIssue("error", f"area {area.get('id')} references unknown entry: {entry_id}"))

    ignored_prefixes = ("ui/dist/", "benchmarks/")
    for path, discovered in symbols.items():
        if path.startswith(ignored_prefixes):
            continue
        covered = covered_symbols_by_path.get(path, set())
        missing = sorted(discovered - covered - covered_symbol_names)
        if missing:
            issues.append(
                MatrixIssue(
                    "warning",
                    f"{path} has public/exported symbols not explicitly mapped: {', '.join(missing[:12])}"
                    + (" ..." if len(missing) > 12 else ""),
                )
            )

    return issues


def enrich_matrix(matrix: dict[str, Any]) -> dict[str, Any]:
    enriched = json.loads(json.dumps(matrix))
    enriched["generated_at"] = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    return enriched


def status_counts(entries: list[dict[str, Any]], path_kind: str) -> Counter[str]:
    return Counter(entry.get("paths", {}).get(path_kind, {}).get("status", "missing") for entry in entries)


def matrix_summary(matrix: dict[str, Any]) -> dict[str, Any]:
    entries = matrix.get("entries", [])
    return {
        "total_entries": len(entries),
        "by_language": dict(Counter(entry.get("language", "unknown") for entry in entries)),
        "by_risk": dict(Counter(entry.get("risk", "unknown") for entry in entries)),
        "happy": dict(status_counts(entries, "happy")),
        "exceptional": dict(status_counts(entries, "exceptional")),
        "concurrency": dict(status_counts(entries, "concurrency")),
    }


def render_markdown(matrix: dict[str, Any], issues: list[MatrixIssue]) -> str:
    entries = matrix.get("entries", [])
    areas = {area["id"]: area["name"] for area in matrix.get("areas", [])}
    lines = [
        "# Test Coverage Matrix",
        "",
        f"Generated: {matrix.get('generated_at')}",
        "",
        "## Summary",
        "",
        f"- Entries: {len(entries)}",
        f"- Languages: {', '.join(f'{key}={value}' for key, value in matrix_summary(matrix)['by_language'].items())}",
        f"- Risks: {', '.join(f'{key}={value}' for key, value in matrix_summary(matrix)['by_risk'].items())}",
        "",
        "## Matrix",
        "",
        "| Area | Symbol / Interface | Module | Risk | Happy | Exceptional | Concurrency | Missing Cases |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
    ]

    for entry in entries:
        paths = entry.get("paths", {})
        missing_cases = []
        for path_kind in REQUIRED_PATH_KINDS:
            missing_cases.extend(paths.get(path_kind, {}).get("missing_cases", []))
        lines.append(
            "| "
            + " | ".join(
                [
                    markdown_cell(areas.get(entry.get("area"), entry.get("area", ""))),
                    markdown_cell(entry.get("symbol", "")),
                    markdown_cell(entry.get("module", "")),
                    markdown_cell(entry.get("risk", "")),
                    markdown_cell(paths.get("happy", {}).get("status", "")),
                    markdown_cell(paths.get("exceptional", {}).get("status", "")),
                    markdown_cell(paths.get("concurrency", {}).get("status", "")),
                    markdown_cell("; ".join(missing_cases)),
                ]
            )
            + " |"
        )

    warnings = [issue for issue in issues if issue.level == "warning"]
    if warnings:
        lines.extend(["", "## Drift Warnings", ""])
        lines.extend(f"- {issue.message}" for issue in warnings)

    return "\n".join(lines) + "\n"


def markdown_cell(value: Any) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ")


def render_html(matrix: dict[str, Any], issues: list[MatrixIssue], markdown: str) -> str:
    payload = {
        "matrix": matrix,
        "summary": matrix_summary(matrix),
        "issues": [{"level": issue.level, "message": issue.message} for issue in issues],
        "markdown": markdown,
    }
    escaped_payload = json.dumps(payload, ensure_ascii=False).replace("</", "<\\/")
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>dirbase Test Coverage Matrix</title>
  <style>
    :root {{
      color-scheme: light;
      --bg: #f7f8fa;
      --panel: #ffffff;
      --border: #d8dee8;
      --text: #18202a;
      --muted: #657184;
      --critical: #b42318;
      --high: #b54708;
      --medium: #175cd3;
      --low: #067647;
      --covered: #067647;
      --partial: #b54708;
      --missing: #b42318;
      --na: #667085;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    header {{
      padding: 24px;
      border-bottom: 1px solid var(--border);
      background: var(--panel);
    }}
    main {{ padding: 20px 24px 32px; }}
    h1 {{ margin: 0 0 6px; font-size: 24px; }}
    h2 {{ margin: 0 0 12px; font-size: 16px; }}
    p {{ margin: 0; color: var(--muted); }}
    .summary {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 12px;
      margin: 20px 0;
    }}
    .card {{
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 14px;
    }}
    .metric {{ font-size: 26px; font-weight: 700; }}
    .label {{ color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .08em; }}
    .toolbar {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 10px;
      align-items: end;
      margin-bottom: 16px;
    }}
    label {{ display: grid; gap: 4px; font-size: 12px; color: var(--muted); }}
    input, select, button {{
      min-height: 36px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: #fff;
      color: var(--text);
      padding: 7px 9px;
      font: inherit;
    }}
    button {{ cursor: pointer; font-weight: 600; }}
    .button-row {{ display: flex; gap: 8px; flex-wrap: wrap; }}
    .table-wrap {{
      overflow-x: auto;
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 8px;
    }}
    table {{ width: 100%; border-collapse: collapse; min-width: 1100px; }}
    th, td {{ padding: 10px 12px; border-bottom: 1px solid var(--border); text-align: left; vertical-align: top; }}
    th {{ font-size: 12px; color: var(--muted); background: #fbfcfe; position: sticky; top: 0; }}
    tr:last-child td {{ border-bottom: 0; }}
    .pill {{
      display: inline-flex;
      align-items: center;
      border-radius: 999px;
      padding: 2px 8px;
      font-size: 12px;
      font-weight: 700;
      background: #eef2f6;
      color: var(--muted);
      white-space: nowrap;
    }}
    .risk-critical {{ color: var(--critical); }}
    .risk-high {{ color: var(--high); }}
    .risk-medium {{ color: var(--medium); }}
    .risk-low {{ color: var(--low); }}
    .status-covered {{ color: var(--covered); }}
    .status-partial {{ color: var(--partial); }}
    .status-missing {{ color: var(--missing); }}
    .status-not_applicable {{ color: var(--na); }}
    details {{ max-width: 420px; }}
    summary {{ cursor: pointer; font-weight: 600; }}
    ul {{ margin: 8px 0 0; padding-left: 18px; }}
    .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; }}
    .issues {{ display: grid; gap: 8px; margin-bottom: 16px; }}
    .issue {{ border-left: 3px solid var(--partial); background: #fff7ed; padding: 10px 12px; border-radius: 6px; }}
    @media (max-width: 720px) {{
      header, main {{ padding-left: 14px; padding-right: 14px; }}
      h1 {{ font-size: 20px; }}
    }}
  </style>
</head>
<body>
  <script id="payload" type="application/json">{escaped_payload}</script>
  <header>
    <h1>dirbase Test Coverage Matrix</h1>
    <p id="generated"></p>
  </header>
  <main>
    <section class="summary" id="summary"></section>
    <section class="card">
      <h2>Filters</h2>
      <div class="toolbar">
        <label>Search <input id="search" type="search" placeholder="symbol, module, test, missing case"></label>
        <label>Language <select id="language"></select></label>
        <label>Area <select id="area"></select></label>
        <label>Risk <select id="risk"></select></label>
        <label>Path status <select id="pathStatus"></select></label>
        <label>Test type <select id="testType"></select></label>
        <label>Module <select id="module"></select></label>
      </div>
      <div class="button-row">
        <button id="reset">Reset Filters</button>
        <button id="downloadJson">Download JSON</button>
        <button id="downloadMarkdown">Download Markdown</button>
      </div>
    </section>
    <section class="issues" id="issues"></section>
    <section class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Area</th>
            <th>Symbol / Interface</th>
            <th>Module</th>
            <th>Risk</th>
            <th>Happy</th>
            <th>Exceptional</th>
            <th>Concurrency</th>
            <th>Covered Tests</th>
            <th>Missing Cases</th>
          </tr>
        </thead>
        <tbody id="rows"></tbody>
      </table>
    </section>
  </main>
  <script>
    const payload = JSON.parse(document.getElementById('payload').textContent);
    const matrix = payload.matrix;
    const entries = matrix.entries || [];
    const areas = new Map((matrix.areas || []).map((area) => [area.id, area.name]));
    const filters = ['language', 'area', 'risk', 'pathStatus', 'testType', 'module'];

    function unique(values) {{
      return [...new Set(values.filter(Boolean))].sort((a, b) => String(a).localeCompare(String(b)));
    }}

    function fillSelect(id, values) {{
      const select = document.getElementById(id);
      select.innerHTML = '<option value="">All</option>' + values.map((value) => `<option value="${{escapeAttr(value)}}">${{escapeHtml(value)}}</option>`).join('');
    }}

    function escapeHtml(value) {{
      return String(value ?? '').replace(/[&<>"']/g, (char) => ({{'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}}[char]));
    }}

    function escapeAttr(value) {{
      return escapeHtml(value);
    }}

    function statusCell(entry, kind) {{
      const status = entry.paths?.[kind]?.status || 'missing';
      return `<span class="pill status-${{status}}">${{escapeHtml(status)}}</span>`;
    }}

    function allMissingCases(entry) {{
      return ['happy', 'exceptional', 'concurrency'].flatMap((kind) => entry.paths?.[kind]?.missing_cases || []);
    }}

    function entryText(entry) {{
      return [
        entry.id, entry.area, areas.get(entry.area), entry.symbol, entry.interface, entry.module, entry.risk,
        ...(entry.covered_symbols || []),
        ...(entry.covered_by || []).flatMap((test) => [test.test, test.path, ...(test.asserts || [])]),
        ...(entry.parameters || []).flatMap((parameter) => [parameter.name, ...(parameter.covered_values || []), ...(parameter.missing_values || [])]),
        ...allMissingCases(entry)
      ].join(' ').toLowerCase();
    }}

    function matches(entry) {{
      const search = document.getElementById('search').value.trim().toLowerCase();
      if (search && !entryText(entry).includes(search)) return false;
      if (document.getElementById('language').value && entry.language !== document.getElementById('language').value) return false;
      if (document.getElementById('area').value && entry.area !== document.getElementById('area').value) return false;
      if (document.getElementById('risk').value && entry.risk !== document.getElementById('risk').value) return false;
      if (document.getElementById('module').value && entry.module !== document.getElementById('module').value) return false;
      const pathStatus = document.getElementById('pathStatus').value;
      if (pathStatus && !['happy', 'exceptional', 'concurrency'].some((kind) => entry.paths?.[kind]?.status === pathStatus)) return false;
      const testType = document.getElementById('testType').value;
      if (testType && !(entry.covered_by || []).some((test) => test.test_type === testType)) return false;
      return true;
    }}

    function renderRows() {{
      const visible = entries.filter(matches);
      document.getElementById('rows').innerHTML = visible.map((entry) => {{
        const missing = allMissingCases(entry);
        const tests = (entry.covered_by || []).map((test) => `<li><span class="mono">${{escapeHtml(test.path)}}</span><br>${{escapeHtml(test.test)}} <span class="pill">${{escapeHtml(test.test_type)}}</span></li>`).join('');
        const params = (entry.parameters || []).map((parameter) => `<li><strong>${{escapeHtml(parameter.name)}}</strong>: covered ${{escapeHtml((parameter.covered_values || []).join(', ') || 'none')}}; missing ${{escapeHtml((parameter.missing_values || []).join(', ') || 'none')}}</li>`).join('');
        return `<tr>
          <td>${{escapeHtml(areas.get(entry.area) || entry.area)}}</td>
          <td><strong>${{escapeHtml(entry.symbol)}}</strong><br><span class="mono">${{escapeHtml(entry.interface || '')}}</span>
            <details><summary>Details</summary><ul>${{params}}</ul><p>${{escapeHtml(entry.notes || '')}}</p></details>
          </td>
          <td class="mono">${{escapeHtml(entry.module)}}</td>
          <td><span class="pill risk-${{entry.risk}}">${{escapeHtml(entry.risk)}}</span></td>
          <td>${{statusCell(entry, 'happy')}}</td>
          <td>${{statusCell(entry, 'exceptional')}}</td>
          <td>${{statusCell(entry, 'concurrency')}}</td>
          <td><ul>${{tests}}</ul></td>
          <td>${{missing.length ? `<ul>${{missing.map((item) => `<li>${{escapeHtml(item)}}</li>`).join('')}}</ul>` : '<span class="pill status-covered">none</span>'}}</td>
        </tr>`;
      }}).join('');
    }}

    function renderSummary() {{
      const summary = payload.summary;
      const cards = [
        ['Entries', summary.total_entries],
        ['Happy covered', summary.happy.covered || 0],
        ['Exceptional covered', summary.exceptional.covered || 0],
        ['Concurrency covered', summary.concurrency.covered || 0],
        ['Critical risk', summary.by_risk.critical || 0],
        ['High risk', summary.by_risk.high || 0]
      ];
      document.getElementById('summary').innerHTML = cards.map(([label, value]) => `<div class="card"><div class="metric">${{value}}</div><div class="label">${{label}}</div></div>`).join('');
      document.getElementById('generated').textContent = `Generated ${{matrix.generated_at || 'from source matrix'}}`;
    }}

    function renderIssues() {{
      const warnings = payload.issues.filter((issue) => issue.level === 'warning');
      document.getElementById('issues').innerHTML = warnings.length
        ? `<section class="card"><h2>Drift Warnings</h2>${{warnings.slice(0, 12).map((issue) => `<div class="issue">${{escapeHtml(issue.message)}}</div>`).join('')}}</section>`
        : '';
    }}

    function download(name, type, content) {{
      const blob = new Blob([content], {{ type }});
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = name;
      link.click();
      URL.revokeObjectURL(url);
    }}

    fillSelect('language', unique(entries.map((entry) => entry.language)));
    fillSelect('area', unique(entries.map((entry) => entry.area)));
    fillSelect('risk', unique(entries.map((entry) => entry.risk)));
    fillSelect('pathStatus', unique(entries.flatMap((entry) => ['happy', 'exceptional', 'concurrency'].map((kind) => entry.paths?.[kind]?.status))));
    fillSelect('testType', unique(entries.flatMap((entry) => (entry.covered_by || []).map((test) => test.test_type))));
    fillSelect('module', unique(entries.map((entry) => entry.module)));

    document.getElementById('search').addEventListener('input', renderRows);
    filters.forEach((id) => document.getElementById(id).addEventListener('change', renderRows));
    document.getElementById('reset').addEventListener('click', () => {{
      document.getElementById('search').value = '';
      filters.forEach((id) => document.getElementById(id).value = '');
      renderRows();
    }});
    document.getElementById('downloadJson').addEventListener('click', () => download('test-matrix.json', 'application/json', JSON.stringify(matrix, null, 2)));
    document.getElementById('downloadMarkdown').addEventListener('click', () => download('test-matrix.md', 'text/markdown', payload.markdown));

    renderSummary();
    renderIssues();
    renderRows();
  </script>
</body>
</html>
"""


def write_outputs(matrix: dict[str, Any], issues: list[MatrixIssue]) -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    enriched = enrich_matrix(matrix)
    markdown = render_markdown(enriched, issues)
    html_output = render_html(enriched, issues, markdown)

    (OUTPUT_DIR / "test-matrix.json").write_text(json.dumps(enriched, indent=2) + "\n", encoding="utf-8")
    (OUTPUT_DIR / "test-matrix.md").write_text(markdown, encoding="utf-8")
    (OUTPUT_DIR / "index.html").write_text(html_output, encoding="utf-8")
    print(f"Wrote {relpath(OUTPUT_DIR / 'index.html')}")
    print(f"Wrote {relpath(OUTPUT_DIR / 'test-matrix.json')}")
    print(f"Wrote {relpath(OUTPUT_DIR / 'test-matrix.md')}")


def serve_output(port: int) -> None:
    os.chdir(OUTPUT_DIR)
    handler = http.server.SimpleHTTPRequestHandler
    with socketserver.TCPServer(("127.0.0.1", port), handler) as httpd:
        print(f"Serving test matrix at http://127.0.0.1:{port}")
        httpd.serve_forever()


def print_issues(issues: list[MatrixIssue]) -> None:
    for issue in issues:
        stream = sys.stderr if issue.level == "error" else sys.stdout
        print(str(issue), file=stream)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate the matrix and fail on errors")
    parser.add_argument("--write", action="store_true", help="write the static report to target/test-matrix")
    parser.add_argument("--serve", action="store_true", help="write and serve the static report")
    parser.add_argument("--port", type=int, default=8765, help="port for --serve")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not (args.check or args.write or args.serve):
        args.check = True

    matrix = load_matrix()
    tests = discover_tests()
    symbols = discover_public_symbols()
    issues = validate_matrix(matrix, tests, symbols)
    print_issues(issues)

    errors = [issue for issue in issues if issue.level == "error"]
    if errors:
        return 1

    if args.write or args.serve:
        write_outputs(matrix, issues)

    if args.check:
        warning_count = sum(1 for issue in issues if issue.level == "warning")
        print(
            f"Validated {len(matrix.get('entries', []))} matrix entries "
            f"with {warning_count} drift warning{'s' if warning_count != 1 else ''}."
        )

    if args.serve:
        serve_output(args.port)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
