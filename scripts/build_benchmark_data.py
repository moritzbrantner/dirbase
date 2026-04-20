#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import shutil
from datetime import datetime, timedelta, timezone
from pathlib import Path

REGIONS = ("us-east", "us-west", "eu-central", "eu-west", "ap-south")
SERVICES = ("payments", "search", "identity", "messaging", "analytics", "storage")
ROLES = ("engineer", "analyst", "manager", "sre", "support")
PROJECT_STATUSES = ("active", "planning", "paused", "delivery")
CUSTOMER_TIERS = ("starter", "growth", "enterprise")
TICKET_STATUSES = ("open", "in_progress", "blocked", "resolved", "closed")
CHANNELS = ("email", "chat", "api", "web")
ENVIRONMENTS = ("dev", "staging", "prod", "preview")
INCIDENT_TERMS = (
    "timeout",
    "latency",
    "quota",
    "cache",
    "replication",
    "auth",
    "deploy",
)


def parse_args() -> argparse.Namespace:
    root_dir = Path(__file__).resolve().parent.parent
    default_output_dir = root_dir / "benchmarks" / ".work" / "benchmark-data"

    parser = argparse.ArgumentParser(
        description="Generate deterministic synthetic benchmark data for dirbase and json-server."
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=default_output_dir,
        help=f"Output directory for generated JSON data (default: {default_output_dir})",
    )
    parser.add_argument(
        "--organization-count",
        type=int,
        default=12,
        help="Number of organizations to generate (default: 12).",
    )
    parser.add_argument(
        "--team-count",
        type=int,
        default=240,
        help="Number of teams to generate (default: 240).",
    )
    parser.add_argument(
        "--member-count",
        type=int,
        default=12000,
        help="Number of members to generate (default: 12000).",
    )
    parser.add_argument(
        "--project-count",
        type=int,
        default=8000,
        help="Number of projects to generate (default: 8000).",
    )
    parser.add_argument(
        "--ticket-count",
        type=int,
        default=48000,
        help="Number of tickets to generate (default: 48000).",
    )
    parser.add_argument(
        "--deployment-count",
        type=int,
        default=24000,
        help="Number of deployments to generate (default: 24000).",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Regenerate even if cached output already matches the requested profile.",
    )
    return parser.parse_args()


def profile_from_args(args: argparse.Namespace) -> dict[str, int]:
    profile = {
        "organizations": args.organization_count,
        "teams": args.team_count,
        "members": args.member_count,
        "projects": args.project_count,
        "tickets": args.ticket_count,
        "deployments": args.deployment_count,
    }
    for name, value in profile.items():
        if value < 1:
            raise SystemExit(f"{name} must be greater than 0")
    return profile


def output_paths(output_dir: Path) -> tuple[Path, Path, Path]:
    folder_dir = output_dir / "folder"
    db_path = output_dir / "db.json"
    metadata_path = output_dir / "metadata.json"
    return folder_dir, db_path, metadata_path


def load_metadata(metadata_path: Path) -> dict | None:
    if not metadata_path.exists():
        return None
    return json.loads(metadata_path.read_text(encoding="utf-8"))


def outputs_are_fresh(metadata_path: Path, folder_dir: Path, db_path: Path, profile: dict[str, int]) -> bool:
    metadata = load_metadata(metadata_path)
    if metadata is None:
        return False
    if metadata.get("profile") != profile:
        return False
    if not folder_dir.is_dir() or not db_path.is_file():
        return False
    expected = set(profile)
    actual = {path.stem for path in folder_dir.glob("*.json")}
    return expected == actual


def iso_timestamp(offset_hours: int) -> str:
    base = datetime(2025, 1, 1, 9, 0, tzinfo=timezone.utc)
    value = base + timedelta(hours=offset_hours)
    return value.isoformat().replace("+00:00", "Z")


def write_json(path: Path, payload: list[dict]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, separators=(",", ":"))


def generate_organizations(count: int) -> list[dict]:
    organizations = []
    for org_id in range(1, count + 1):
        organizations.append(
            {
                "id": org_id,
                "slug": f"org-{org_id:02d}",
                "name": f"Organization {org_id:02d}",
                "region_hq": REGIONS[(org_id - 1) % len(REGIONS)],
                "plan": CUSTOMER_TIERS[(org_id - 1) % len(CUSTOMER_TIERS)],
                "active": org_id % 6 != 0,
                "budget_k": 400 + (org_id * 85),
            }
        )
    return organizations


def generate_teams(count: int, organization_count: int) -> list[dict]:
    teams = []
    for team_id in range(1, count + 1):
        teams.append(
            {
                "id": team_id,
                "organization_id": ((team_id - 1) % organization_count) + 1,
                "slug": f"team-{team_id:03d}",
                "name": f"Team {team_id:03d}",
                "region": REGIONS[(team_id - 1) % len(REGIONS)],
                "service": SERVICES[(team_id - 1) % len(SERVICES)],
                "active": team_id % 7 != 0,
                "headcount": 6 + ((team_id * 3) % 18),
                "oncall_rotation": 1 + (team_id % 4),
            }
        )
    return teams


def generate_members(count: int, team_count: int) -> list[dict]:
    members = []
    for member_id in range(1, count + 1):
        team_id = ((member_id - 1) % team_count) + 1
        members.append(
            {
                "id": member_id,
                "team_id": team_id,
                "username": f"user-{member_id:05d}",
                "role": ROLES[(member_id - 1) % len(ROLES)],
                "level": 1 + (member_id % 6),
                "region": REGIONS[(team_id - 1) % len(REGIONS)],
                "active": member_id % 9 != 0,
                "remote": member_id % 3 != 0,
                "tenure_years": round(0.5 + ((member_id * 17) % 120) / 10, 1),
                "tickets_closed": (member_id * 13) % 400,
            }
        )
    return members


def generate_projects(count: int, team_count: int) -> list[dict]:
    projects = []
    for project_id in range(1, count + 1):
        team_id = ((project_id - 1) % team_count) + 1
        projects.append(
            {
                "id": project_id,
                "team_id": team_id,
                "code": f"PRJ-{project_id:05d}",
                "title": f"{SERVICES[(project_id - 1) % len(SERVICES)].title()} Project {project_id:05d}",
                "status": PROJECT_STATUSES[(project_id - 1) % len(PROJECT_STATUSES)],
                "priority": 1 + ((project_id * 7) % 5),
                "budget_k": 80 + ((project_id * 19) % 700),
                "risk_score": (project_id * 13) % 100,
                "customer_tier": CUSTOMER_TIERS[(project_id - 1) % len(CUSTOMER_TIERS)],
                "region": REGIONS[(team_id - 1) % len(REGIONS)],
                "active": project_id % 8 != 0,
                "updated_at": iso_timestamp(project_id * 3),
            }
        )
    return projects


def generate_tickets(count: int, project_count: int, team_count: int, member_count: int) -> list[dict]:
    tickets = []
    for ticket_id in range(1, count + 1):
        project_id = ((ticket_id - 1) % project_count) + 1
        team_id = ((project_id - 1) % team_count) + 1
        term = INCIDENT_TERMS[(ticket_id - 1) % len(INCIDENT_TERMS)]
        tickets.append(
            {
                "id": ticket_id,
                "project_id": project_id,
                "team_id": team_id,
                "assignee_id": ((ticket_id * 11) % member_count) + 1,
                "status": TICKET_STATUSES[(ticket_id - 1) % len(TICKET_STATUSES)],
                "priority": 1 + ((ticket_id * 7) % 5),
                "severity": 1 + ((ticket_id * 5) % 4),
                "estimate_hours": 1 + ((ticket_id * 3) % 40),
                "sla_breach": ticket_id % 11 == 0,
                "channel": CHANNELS[(ticket_id - 1) % len(CHANNELS)],
                "region": REGIONS[(team_id - 1) % len(REGIONS)],
                "summary": f"{term} alert on service lane {ticket_id % 97}",
                "opened_at": iso_timestamp(ticket_id),
                "due_at": iso_timestamp(ticket_id + 96),
            }
        )
    return tickets


def generate_deployments(count: int, project_count: int, team_count: int) -> list[dict]:
    deployments = []
    for deployment_id in range(1, count + 1):
        project_id = ((deployment_id - 1) % project_count) + 1
        team_id = ((project_id - 1) % team_count) + 1
        success = deployment_id % 6 != 0
        rollback = not success and deployment_id % 2 == 0
        deployments.append(
            {
                "id": deployment_id,
                "project_id": project_id,
                "team_id": team_id,
                "environment": ENVIRONMENTS[(deployment_id - 1) % len(ENVIRONMENTS)],
                "region": REGIONS[(deployment_id - 1) % len(REGIONS)],
                "success": success,
                "rollback": rollback,
                "duration_ms": 120 + ((deployment_id * 37) % 9000),
                "lead_time_hours": 1 + ((deployment_id * 5) % 48),
                "approval_gate": deployment_id % 4 == 0,
                "summary": f"{'rollback' if rollback else 'deploy'} batch {deployment_id % 113}",
                "started_at": iso_timestamp(deployment_id * 2),
                "ended_at": iso_timestamp((deployment_id * 2) + 2),
            }
        )
    return deployments


def build_outputs(output_dir: Path, profile: dict[str, int]) -> dict:
    folder_dir, db_path, metadata_path = output_paths(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    folder_dir.mkdir(parents=True, exist_ok=True)
    for stale_file in folder_dir.glob("*.json"):
        stale_file.unlink()

    resources = [
        ("organizations", generate_organizations(profile["organizations"])),
        ("teams", generate_teams(profile["teams"], profile["organizations"])),
        ("members", generate_members(profile["members"], profile["teams"])),
        ("projects", generate_projects(profile["projects"], profile["teams"])),
        (
            "tickets",
            generate_tickets(
                profile["tickets"],
                profile["projects"],
                profile["teams"],
                profile["members"],
            ),
        ),
        (
            "deployments",
            generate_deployments(profile["deployments"], profile["projects"], profile["teams"]),
        ),
    ]

    total_rows = 0
    resource_metadata = []
    with db_path.open("w", encoding="utf-8") as db_handle:
        db_handle.write("{")
        first = True
        for resource_name, rows in resources:
            resource_path = folder_dir / f"{resource_name}.json"
            write_json(resource_path, rows)
            if not first:
                db_handle.write(",")
            db_handle.write(json.dumps(resource_name))
            db_handle.write(":")
            with resource_path.open("r", encoding="utf-8") as resource_handle:
                shutil.copyfileobj(resource_handle, db_handle)
            first = False

            total_rows += len(rows)
            resource_metadata.append(
                {
                    "name": resource_name,
                    "rows": len(rows),
                    "json": str(resource_path),
                }
            )
        db_handle.write("}")

    focus_team_id = min(17, profile["teams"])
    focus_region = REGIONS[(focus_team_id - 1) % len(REGIONS)]
    scenario_values = {
        "ticket_item_id": max(1, profile["tickets"] // 2),
        "project_item_id": max(1, profile["projects"] // 2),
        "deployment_item_id": max(1, profile["deployments"] // 2),
        "focus_team_id": focus_team_id,
        "focus_region": focus_region,
        "summary_term": "timeout",
        "project_risk_threshold": 70,
        "project_budget_ceiling_k": 400,
        "priority_threshold": 4,
        "page_size": 100,
    }

    metadata = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC"),
        "dataset_name": "synthetic-workload",
        "profile": profile,
        "output_dir": str(output_dir),
        "folder_dir": str(folder_dir),
        "db_path": str(db_path),
        "resource_count": len(resources),
        "total_rows": total_rows,
        "resources": resource_metadata,
        "scenario_values": scenario_values,
    }
    metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    return metadata


def main() -> None:
    args = parse_args()
    output_dir = args.output_dir.resolve()
    profile = profile_from_args(args)

    folder_dir, db_path, metadata_path = output_paths(output_dir)
    if not args.force and outputs_are_fresh(metadata_path, folder_dir, db_path, profile):
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

    metadata = build_outputs(output_dir, profile)
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
