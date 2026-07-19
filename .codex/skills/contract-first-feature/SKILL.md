# Contract-First Feature

Use whenever a change crosses React, Rust, and Python boundaries.

1. Modify canonical schema first.
2. Generate or update TS/Rust/Python types.
3. Add compatibility and serialization tests.
4. Implement producer and consumer behind the contract.
5. Ensure errors use the shared envelope.
6. Ensure long work returns a job ID and emits ordered events.
7. Verify cancellation and retry semantics.
8. Update IPC documentation and context pack.
