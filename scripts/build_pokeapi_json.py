#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import json
import re
import shutil
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

INT_RE = re.compile(r"^[+-]?\d+$")
FLOAT_RE = re.compile(r"^[+-]?(?:\d+\.\d*|\.\d+|\d+)(?:[eE][+-]?\d+)?$")
BOOLEANISH_PREFIXES = ("is_", "has_", "can_", "forms_")
BOOLEANISH_NAMES = {"official", "mastery"}


@dataclass
class ColumnStats:
    name: str
    non_empty: int = 0
    all_int: bool = True
    all_float: bool = True
    bool_candidate: bool = False

    def observe(self, raw: str) -> None:
        if raw == "":
            return

        self.non_empty += 1
        if not INT_RE.match(raw):
            self.all_int = False
        if not FLOAT_RE.match(raw):
            self.all_float = False
        if self.bool_candidate and raw not in {"0", "1"}:
            self.bool_candidate = False

    def resolved_type(self) -> str:
        if self.non_empty == 0:
            return "string"
        if self.bool_candidate and self.all_int:
            return "bool"
        if self.all_int:
            return "int"
        if self.all_float:
            return "float"
        return "string"


def parse_args() -> argparse.Namespace:
    root_dir = Path(__file__).resolve().parent.parent
    default_csv_dir = root_dir / "benchmarks" / "pokeapi" / "data" / "v2" / "csv"
    default_output_dir = root_dir / "benchmarks" / ".work" / "pokeapi-json"

    parser = argparse.ArgumentParser(
        description="Convert PokeAPI CSV resources into JSON collections for folder-server and json-server."
    )
    parser.add_argument(
        "--csv-dir",
        type=Path,
        default=default_csv_dir,
        help=f"Directory containing PokeAPI CSV files (default: {default_csv_dir})",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=default_output_dir,
        help=f"Output directory for generated JSON data (default: {default_output_dir})",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if cached output appears current.",
    )
    return parser.parse_args()


def booleanish(name: str) -> bool:
    return name.startswith(BOOLEANISH_PREFIXES) or name in BOOLEANISH_NAMES


def csv_files(csv_dir: Path) -> list[Path]:
    return sorted(csv_dir.glob("*.csv"))


def latest_source_mtime_ns(paths: list[Path]) -> int:
    return max((path.stat().st_mtime_ns for path in paths), default=0)


def output_paths(output_dir: Path) -> tuple[Path, Path, Path]:
    folder_dir = output_dir / "folder"
    db_path = output_dir / "db.json"
    metadata_path = output_dir / "metadata.json"
    return folder_dir, db_path, metadata_path


def load_metadata(metadata_path: Path) -> dict | None:
    if not metadata_path.exists():
        return None
    return json.loads(metadata_path.read_text(encoding="utf-8"))


def outputs_are_fresh(
    metadata_path: Path,
    folder_dir: Path,
    db_path: Path,
    source_files: list[Path],
    latest_mtime_ns: int,
) -> bool:
    metadata = load_metadata(metadata_path)
    if metadata is None:
        return False
    if metadata.get("latest_source_mtime_ns") != latest_mtime_ns:
        return False
    if metadata.get("resource_count") != len(source_files):
        return False
    if not folder_dir.is_dir() or not db_path.is_file():
        return False

    expected = {path.stem for path in source_files}
    actual = {path.stem for path in folder_dir.glob("*.json")}
    return expected == actual


def infer_resource(csv_path: Path) -> dict:
    with csv_path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        fieldnames = list(reader.fieldnames or [])
        stats = {
            field: ColumnStats(name=field, bool_candidate=booleanish(field)) for field in fieldnames
        }
        row_count = 0

        for row in reader:
            row_count += 1
            for field in fieldnames:
                stats[field].observe(row.get(field, ""))

    column_types = {field: stats[field].resolved_type() for field in fieldnames}
    return {
        "fieldnames": fieldnames,
        "row_count": row_count,
        "column_types": column_types,
        "has_id": "id" in column_types,
    }


def convert_value(raw: str, column_type: str):
    if raw == "":
        return None
    if column_type == "bool":
        return raw == "1"
    if column_type == "int":
        return int(raw)
    if column_type == "float":
        return float(raw)
    return raw


def stream_resource_json(csv_path: Path, fieldnames: list[str], column_types: dict[str, str], output: Path) -> None:
    with csv_path.open("r", encoding="utf-8", newline="") as source, output.open(
        "w", encoding="utf-8"
    ) as target:
        reader = csv.DictReader(source)
        target.write("[")
        first = True
        for row in reader:
            record = {
                field: convert_value(row.get(field, ""), column_types[field]) for field in fieldnames
            }
            if not first:
                target.write(",")
            json.dump(record, target, ensure_ascii=False, separators=(",", ":"))
            first = False
        target.write("]")


def build_outputs(source_files: list[Path], output_dir: Path) -> dict:
    folder_dir, db_path, metadata_path = output_paths(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    folder_dir.mkdir(parents=True, exist_ok=True)
    for stale_file in folder_dir.glob("*.json"):
        stale_file.unlink()

    resources = []
    total_rows = 0
    total_columns = 0
    with_id_resources = 0

    with db_path.open("w", encoding="utf-8") as db_handle:
        db_handle.write("{")
        first_resource = True

        for csv_path in source_files:
            resource_name = csv_path.stem
            inferred = infer_resource(csv_path)
            resource_output = folder_dir / f"{resource_name}.json"
            stream_resource_json(
                csv_path, inferred["fieldnames"], inferred["column_types"], resource_output
            )

            if not first_resource:
                db_handle.write(",")
            db_handle.write(json.dumps(resource_name))
            db_handle.write(":")
            with resource_output.open("r", encoding="utf-8") as resource_handle:
                shutil.copyfileobj(resource_handle, db_handle)
            first_resource = False

            total_rows += inferred["row_count"]
            total_columns += len(inferred["fieldnames"])
            with_id_resources += int(inferred["has_id"])
            resources.append(
                {
                    "name": resource_name,
                    "csv": str(csv_path),
                    "json": str(resource_output),
                    "rows": inferred["row_count"],
                    "columns": len(inferred["fieldnames"]),
                    "has_id": inferred["has_id"],
                    "column_types": inferred["column_types"],
                }
            )

        db_handle.write("}")

    metadata = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC"),
        "source_dir": str(source_files[0].parent if source_files else ""),
        "output_dir": str(output_dir),
        "folder_dir": str(folder_dir),
        "db_path": str(db_path),
        "resource_count": len(resources),
        "with_id_resource_count": with_id_resources,
        "total_rows": total_rows,
        "average_columns_per_resource": (total_columns / len(resources)) if resources else 0.0,
        "latest_source_mtime_ns": latest_source_mtime_ns(source_files),
        "resources": resources,
    }
    metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    return metadata


def main() -> None:
    args = parse_args()
    source_dir = args.csv_dir.resolve()
    output_dir = args.output_dir.resolve()
    source_files = csv_files(source_dir)

    if not source_files:
        raise SystemExit(f"No CSV files found in {source_dir}")

    folder_dir, db_path, metadata_path = output_paths(output_dir)
    latest_mtime_ns = latest_source_mtime_ns(source_files)
    if not args.force and outputs_are_fresh(
        metadata_path, folder_dir, db_path, source_files, latest_mtime_ns
    ):
        metadata = load_metadata(metadata_path) or {}
        print(
            json.dumps(
                {
                    "status": "cached",
                    "output_dir": str(output_dir),
                    "db_path": str(db_path),
                    "folder_dir": str(folder_dir),
                    "resource_count": metadata.get("resource_count"),
                    "total_rows": metadata.get("total_rows"),
                }
            )
        )
        return

    metadata = build_outputs(source_files, output_dir)
    print(
        json.dumps(
            {
                "status": "generated",
                "output_dir": str(output_dir),
                "db_path": metadata["db_path"],
                "folder_dir": metadata["folder_dir"],
                "resource_count": metadata["resource_count"],
                "total_rows": metadata["total_rows"],
            }
        )
    )


if __name__ == "__main__":
    main()
