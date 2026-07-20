"""Safe structured runtime logging shared with native and desktop layers."""

from __future__ import annotations

import json
from dataclasses import asdict
from datetime import UTC, datetime
from typing import BinaryIO, Final, Literal
from uuid import UUID

from teratai_engine.generated.runtime_log_event import RuntimeLogEvent

LogLevel = Literal["DEBUG", "ERROR", "INFO", "WARNING"]
RuntimeLayer = Literal["desktop", "engine", "native"]
ALLOWED_LEVELS: Final = frozenset({"DEBUG", "ERROR", "INFO", "WARNING"})
ALLOWED_LAYERS: Final = frozenset({"desktop", "engine", "native"})


def _utc_timestamp() -> str:
    return datetime.now(UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def build_runtime_log(
    *,
    level: LogLevel,
    layer: RuntimeLayer,
    component: str,
    event: str,
    message: str,
    correlation_id: str,
    sequence: int,
    timestamp: str | None = None,
) -> RuntimeLogEvent:
    """Build a validated event containing metadata only, never source-row data."""
    if level not in ALLOWED_LEVELS or layer not in ALLOWED_LAYERS:
        raise ValueError("runtime log level or layer is invalid")
    if not component.strip() or not event.strip() or not message.strip():
        raise ValueError("runtime log string fields cannot be empty")
    if sequence < 1:
        raise ValueError("runtime log sequence must be positive")
    parsed_id = UUID(correlation_id)
    if parsed_id.version != 7:
        raise ValueError("runtime log correlation_id must be UUID v7")

    timestamp_value = timestamp or _utc_timestamp()
    if not timestamp_value.endswith("Z"):
        raise ValueError("runtime log timestamp must be ISO-8601 UTC")
    try:
        datetime.fromisoformat(timestamp_value.removesuffix("Z") + "+00:00")
    except ValueError as error:
        raise ValueError("runtime log timestamp must be ISO-8601 UTC") from error

    return RuntimeLogEvent(
        component=component,
        correlation_id=correlation_id,
        event=event,
        layer=layer,
        level=level,
        message=message,
        sequence=sequence,
        timestamp=timestamp_value,
    )


def emit_runtime_log(stream: BinaryIO, log_event: RuntimeLogEvent) -> None:
    """Write exactly one deterministic JSON event and flush the process boundary."""
    encoded = json.dumps(asdict(log_event), ensure_ascii=False, separators=(",", ":"))
    stream.write(encoded.encode("utf-8") + b"\n")
    stream.flush()
