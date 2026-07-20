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


def serve(input_stream: BinaryIO, output_stream: BinaryIO) -> int:
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
            response = build_handshake_response(request)
        except ProtocolValidationError as error:
            response = build_error(error)
        except Exception as error:  # Defensive process boundary; response remains user-safe.
            response = build_internal_error(error)

        output_stream.write(encode_message(response) + b"\n")
        output_stream.flush()
    return 0


def main() -> int:
    """Run the sidecar using binary streams to enforce byte-size limits."""
    return serve(sys.stdin.buffer, sys.stdout.buffer)


if __name__ == "__main__":
    raise SystemExit(main())
