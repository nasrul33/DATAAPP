import json
from dataclasses import asdict
from pathlib import Path

from teratai_engine.generated.contract_metadata import ContractMetadata


def test_generated_contract_metadata_round_trips_shared_fixture() -> None:
    fixture_path = Path("packages/contracts/fixtures/contract-metadata.valid.json")
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    metadata = ContractMetadata(**fixture)

    assert asdict(metadata) == fixture
