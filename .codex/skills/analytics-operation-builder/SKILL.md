# Analytics Operation Builder

Use for every new transform, statistic, anomaly detector, or scoring operation.

## Required sequence
1. Read AGENTS.md, PRIMITIVES.md, IPC contracts, and current operation registry.
2. Define method, inputs, parameters, output schema, null policy, precision, deterministic behavior, warnings, and limitations.
3. Create a minimal golden dataset and independently verified expected output before implementation.
4. Implement analytics core as a pure/testable unit; keep transport and persistence outside it.
5. Register operation schema and typed contracts.
6. Add engine integration, job progress/cancellation, Rust adapter, then UI configuration.
7. Add unit, property/edge, contract, integration, and golden tests.
8. Benchmark representative data and document resource behavior.
9. Update context pack and artifact catalog references.

## Prohibited
- arbitrary eval;
- silent coercion;
- nondeterministic output without stored seed;
- detector result without explanation/reason codes;
- claiming correctness without golden cross-check.
