import json
from dataclasses import asdict
from pathlib import Path

from teratai_engine.generated.contract_metadata import ContractMetadata
from teratai_engine.generated.job_descriptor import JobDescriptor
from teratai_engine.generated.job_lifecycle_event import JobLifecycleEvent


def test_generated_contract_metadata_round_trips_shared_fixture() -> None:
    fixture_path = Path("packages/contracts/fixtures/contract-metadata.valid.json")
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    metadata = ContractMetadata(**fixture)

    assert asdict(metadata) == fixture


def test_generated_reference_contract_composes_job_descriptor() -> None:
    job = JobDescriptor(
        correlation_id="00000000-0000-7000-8000-000000000403",
        created_at="2026-07-24T01:00:00Z",
        job_id="00000000-0000-7000-8000-000000000402",
        kind="system.mock_long",
        progress_current=0,
        project_id="00000000-0000-7000-8000-000000000401",
        revision=1,
        status="QUEUED",
        updated_at="2026-07-24T01:00:00Z",
    )
    event = JobLifecycleEvent(
        event_name="job.queued",
        job=job,
        occurred_at=job.updated_at,
        protocol_version="1.0",
        sequence=job.revision,
    )

    assert asdict(event)["job"]["job_id"] == job.job_id
