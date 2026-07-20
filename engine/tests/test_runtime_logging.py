from __future__ import annotations

import json
from io import BytesIO

import pytest

from teratai_engine.runtime_logging import build_runtime_log, emit_runtime_log

CORRELATION_ID = "00000000-0000-7000-8000-000000000007"


def test_runtime_log_is_safe_deterministic_json() -> None:
    stream = BytesIO()
    event = build_runtime_log(
        level="INFO",
        layer="engine",
        component="sidecar",
        event="engine.handshake.received",
        message="Engine menerima permintaan handshake.",
        correlation_id=CORRELATION_ID,
        sequence=2,
        timestamp="2026-07-20T10:00:00.000Z",
    )

    emit_runtime_log(stream, event)

    assert json.loads(stream.getvalue()) == {
        "component": "sidecar",
        "correlation_id": CORRELATION_ID,
        "event": "engine.handshake.received",
        "layer": "engine",
        "level": "INFO",
        "message": "Engine menerima permintaan handshake.",
        "sequence": 2,
        "timestamp": "2026-07-20T10:00:00.000Z",
    }


def test_runtime_log_rejects_non_uuid_v7_correlation() -> None:
    with pytest.raises(ValueError, match="UUID v7"):
        build_runtime_log(
            level="INFO",
            layer="engine",
            component="sidecar",
            event="engine.handshake.received",
            message="Engine menerima permintaan handshake.",
            correlation_id="00000000-0000-4000-8000-000000000007",
            sequence=2,
        )
