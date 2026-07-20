"""Bounded and typed lifecycle protocol for the Python engine sidecar."""

from __future__ import annotations

import json
import re
import sys
from dataclasses import asdict
from typing import Final
from uuid import UUID

from teratai_engine import __version__
from teratai_engine.generated.engine_error import EngineError
from teratai_engine.generated.engine_handshake_request import EngineHandshakeRequest
from teratai_engine.generated.engine_handshake_response import EngineHandshakeResponse

ENGINE_PROTOCOL_VERSION: Final = "1.0"
HANDSHAKE_COMMAND: Final = "engine.handshake"
MAX_MESSAGE_BYTES: Final = 64 * 1024
ENGINE_CAPABILITIES: Final = ("health",)
SEMANTIC_VERSION_PATTERN: Final = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


class ProtocolValidationError(ValueError):
    """A request failed bounded wire validation."""

    def __init__(self, detail: str, field_errors: tuple[str, ...], correlation_id: str) -> None:
        super().__init__(detail)
        self.detail = detail
        self.field_errors = field_errors
        self.correlation_id = correlation_id


def _required_string(payload: dict[str, object], field: str) -> str:
    value = payload.get(field)
    if not isinstance(value, str) or not value.strip():
        raise ProtocolValidationError(
            f"{field} must be a non-empty string",
            (f"{field} tidak valid",),
            _correlation_id(payload),
        )
    return value


def _correlation_id(payload: dict[str, object]) -> str:
    value = payload.get("request_id")
    return value if isinstance(value, str) and value else "engine-startup"


def _request_id(payload: dict[str, object]) -> str:
    value = _required_string(payload, "request_id")
    try:
        parsed = UUID(value)
    except ValueError as error:
        raise ProtocolValidationError(
            "request_id must be a UUID",
            ("request_id bukan UUID yang valid",),
            "engine-startup",
        ) from error
    if parsed.version != 7:
        raise ProtocolValidationError(
            "request_id must use UUID version 7",
            ("request_id harus menggunakan UUID v7",),
            "engine-startup",
        )
    return value


def decode_handshake_request(message: bytes) -> EngineHandshakeRequest:
    """Decode one bounded handshake request and reject ambiguous input."""
    if not message or len(message) > MAX_MESSAGE_BYTES:
        raise ProtocolValidationError(
            "message size is outside the accepted bounds",
            ("message harus berukuran 1 sampai 65536 byte",),
            "engine-startup",
        )

    try:
        decoded = json.loads(message)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ProtocolValidationError(
            "message is not valid UTF-8 JSON",
            ("message bukan JSON UTF-8 yang valid",),
            "engine-startup",
        ) from error

    if not isinstance(decoded, dict) or not all(isinstance(key, str) for key in decoded):
        raise ProtocolValidationError(
            "message root must be a JSON object",
            ("message harus berupa object",),
            "engine-startup",
        )

    payload: dict[str, object] = decoded
    host_version = _required_string(payload, "host_version")
    if SEMANTIC_VERSION_PATTERN.fullmatch(host_version) is None:
        raise ProtocolValidationError(
            "host_version must use semantic version format",
            ("host_version tidak valid",),
            _correlation_id(payload),
        )

    request = EngineHandshakeRequest(
        protocol_version=_required_string(payload, "protocol_version"),
        request_id=_request_id(payload),
        command=_required_string(payload, "command"),
        host_version=host_version,
    )
    if request.command != HANDSHAKE_COMMAND:
        raise ProtocolValidationError(
            f"unsupported lifecycle command: {request.command}",
            ("command tidak didukung",),
            request.request_id,
        )
    if request.protocol_version != ENGINE_PROTOCOL_VERSION:
        raise ProtocolValidationError(
            f"expected protocol {ENGINE_PROTOCOL_VERSION}, received {request.protocol_version}",
            ("protocol_version tidak didukung",),
            request.request_id,
        )
    return request


def build_handshake_response(request: EngineHandshakeRequest) -> EngineHandshakeResponse:
    """Return engine identity only when the required Python runtime is active."""
    python_version = ".".join(str(part) for part in sys.version_info[:3])
    healthy = sys.version_info[:2] == (3, 12)
    return EngineHandshakeResponse(
        capabilities=list(ENGINE_CAPABILITIES),
        engine_version=__version__,
        healthy=healthy,
        protocol_version=ENGINE_PROTOCOL_VERSION,
        python_version=python_version,
        request_id=request.request_id,
        status="ready" if healthy else "incompatible-runtime",
    )


def build_error(error: ProtocolValidationError) -> EngineError:
    """Map validation failures into the canonical safe error envelope."""
    return EngineError(
        code="ENGINE_UNAVAILABLE",
        message="Engine tidak dapat menyelesaikan handshake.",
        detail=error.detail,
        retriable=False,
        correlation_id=error.correlation_id,
        field_errors=list(error.field_errors),
    )


def build_internal_error(error: Exception) -> EngineError:
    """Return a safe envelope for an unexpected engine boundary failure."""
    return EngineError(
        code="OPERATION_FAILED",
        message="Engine mengalami kegagalan internal saat memproses handshake.",
        detail=f"unexpected {type(error).__name__}",
        retriable=False,
        correlation_id="engine-startup",
        field_errors=[],
    )


def encode_message(message: EngineError | EngineHandshakeResponse) -> bytes:
    """Serialize a protocol dataclass deterministically as one JSON line."""
    return json.dumps(asdict(message), ensure_ascii=False, separators=(",", ":")).encode("utf-8")
