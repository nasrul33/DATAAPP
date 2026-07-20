from __future__ import annotations

import json
import subprocess
import sys
from io import BytesIO
from pathlib import Path

from teratai_engine.protocol import (
    ENGINE_PROTOCOL_VERSION,
    ProtocolValidationError,
    build_handshake_response,
    decode_handshake_request,
)
from teratai_engine.sidecar import serve

REQUEST_ID = "00000000-0000-7000-8000-000000000006"


def handshake_payload(protocol_version: str = ENGINE_PROTOCOL_VERSION) -> dict[str, object]:
    return {
        "protocol_version": protocol_version,
        "request_id": REQUEST_ID,
        "command": "engine.handshake",
        "host_version": "0.1.0",
    }


def test_handshake_reports_protocol_version_and_health() -> None:
    request = decode_handshake_request(json.dumps(handshake_payload()).encode())
    response = build_handshake_response(request)

    assert response.protocol_version == "1.0"
    assert response.engine_version == "0.1.0"
    assert response.python_version.startswith("3.12.")
    assert response.healthy is True
    assert response.status == "ready"
    assert response.capabilities == ["health"]


def test_protocol_mismatch_returns_typed_error_envelope() -> None:
    input_stream = BytesIO(json.dumps(handshake_payload("2.0")).encode() + b"\n")
    output_stream = BytesIO()

    assert serve(input_stream, output_stream) == 0
    error = json.loads(output_stream.getvalue())

    assert error["code"] == "ENGINE_UNAVAILABLE"
    assert error["correlation_id"] == REQUEST_ID
    assert error["retriable"] is False
    assert error["field_errors"] == ["protocol_version tidak didukung"]


def test_oversized_message_is_rejected() -> None:
    oversized = b"{" + (b"x" * (64 * 1024)) + b"}"

    try:
        decode_handshake_request(oversized)
    except ProtocolValidationError as error:
        assert "size" in error.detail
    else:
        raise AssertionError("oversized message must be rejected")


def test_non_uuid_v7_request_id_is_rejected_without_echoing_it() -> None:
    payload = handshake_payload()
    payload["request_id"] = "00000000-0000-4000-8000-000000000006"

    try:
        decode_handshake_request(json.dumps(payload).encode())
    except ProtocolValidationError as error:
        assert error.correlation_id == "engine-startup"
        assert error.field_errors == ("request_id harus menggunakan UUID v7",)
    else:
        raise AssertionError("non-UUID-v7 correlation identifiers must be rejected")


def test_invalid_correlation_cannot_break_typed_error_response() -> None:
    payload = handshake_payload()
    payload["host_version"] = "invalid"
    payload["request_id"] = "not-a-uuid"
    output_stream = BytesIO()
    error_stream = BytesIO()

    assert serve(
        BytesIO(json.dumps(payload).encode() + b"\n"),
        output_stream,
        error_stream,
    ) == 0
    response = json.loads(output_stream.getvalue())

    assert response["code"] == "ENGINE_UNAVAILABLE"
    assert response["correlation_id"] == "not-a-uuid"
    assert error_stream.getvalue() == b""


def test_module_sidecar_stays_alive_until_host_closes_stdin() -> None:
    process = subprocess.Popen(
        [sys.executable, "-B", "-m", "teratai_engine.sidecar"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        cwd=Path(__file__).resolve().parents[1],
    )
    assert process.stdin is not None
    assert process.stdout is not None

    process.stdin.write(json.dumps(handshake_payload()) + "\n")
    process.stdin.flush()
    response = json.loads(process.stdout.readline())
    assert response["healthy"] is True
    assert process.poll() is None

    assert process.stderr is not None
    received_log = json.loads(process.stderr.readline())
    ready_log = json.loads(process.stderr.readline())
    assert [received_log["sequence"], ready_log["sequence"]] == [2, 3]
    assert received_log["correlation_id"] == REQUEST_ID
    assert ready_log["correlation_id"] == REQUEST_ID

    process.stdin.close()
    assert process.wait(timeout=5) == 0
