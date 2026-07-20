"""Standard-input/standard-output entrypoint for the Teratai engine sidecar."""

from __future__ import annotations

import sys
from typing import BinaryIO

from teratai_engine.generated.engine_error import EngineError
from teratai_engine.generated.engine_handshake_response import EngineHandshakeResponse
from teratai_engine.protocol import (
    MAX_MESSAGE_BYTES,
    ProtocolValidationError,
    build_error,
    build_handshake_response,
    build_internal_error,
    decode_handshake_request,
    encode_message,
)
from teratai_engine.runtime_logging import build_runtime_log, emit_runtime_log


def serve(
    input_stream: BinaryIO,
    output_stream: BinaryIO,
    error_stream: BinaryIO | None = None,
) -> int:
    """Serve bounded lifecycle messages until the native host closes stdin."""
    while message := input_stream.readline(MAX_MESSAGE_BYTES + 2):
        if len(message) > MAX_MESSAGE_BYTES and not message.endswith((b"\n", b"\r")):
            while remainder := input_stream.readline(MAX_MESSAGE_BYTES + 2):
                if remainder.endswith((b"\n", b"\r")):
                    break
        raw_message = message.rstrip(b"\r\n")
        response: EngineError | EngineHandshakeResponse
        try:
            request = decode_handshake_request(raw_message)
            if error_stream is not None:
                emit_runtime_log(
                    error_stream,
                    build_runtime_log(
                        level="INFO",
                        layer="engine",
                        component="sidecar",
                        event="engine.handshake.received",
                        message="Engine menerima permintaan handshake.",
                        correlation_id=request.request_id,
                        sequence=2,
                    ),
                )
            response = build_handshake_response(request)
            if error_stream is not None:
                emit_runtime_log(
                    error_stream,
                    build_runtime_log(
                        level="INFO" if response.healthy else "ERROR",
                        layer="engine",
                        component="sidecar",
                        event=(
                            "engine.handshake.ready"
                            if response.healthy
                            else "engine.handshake.unhealthy"
                        ),
                        message=(
                            "Engine siap menerima operasi."
                            if response.healthy
                            else "Engine gagal health check."
                        ),
                        correlation_id=request.request_id,
                        sequence=3,
                    ),
                )
        except ProtocolValidationError as error:
            response = build_error(error)
            if error_stream is not None and error.correlation_id != "engine-startup":
                try:
                    rejected_log = build_runtime_log(
                        level="WARNING",
                        layer="engine",
                        component="sidecar",
                        event="engine.handshake.rejected",
                        message="Engine menolak permintaan handshake.",
                        correlation_id=error.correlation_id,
                        sequence=2,
                    )
                except ValueError:
                    pass
                else:
                    emit_runtime_log(error_stream, rejected_log)
        except Exception as error:  # Defensive process boundary; response remains user-safe.
            response = build_internal_error(error)

        output_stream.write(encode_message(response) + b"\n")
        output_stream.flush()
    return 0


def main() -> int:
    """Run the sidecar using binary streams to enforce byte-size limits."""
    return serve(sys.stdin.buffer, sys.stdout.buffer, sys.stderr.buffer)


if __name__ == "__main__":
    raise SystemExit(main())
