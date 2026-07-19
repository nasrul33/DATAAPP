# Artifact Catalog

Artifacts adalah keluaran terkontrol yang menjaga konsistensi produk dan pekerjaan Codex.

| ID | Artifact | Path | Owner | Update trigger | Gate |
|---|---|---|---|---|---|
| ART-001 | Product definition | `docs/PRODUCT.md` | Product | Scope/metric berubah | Review scope |
| ART-002 | Architecture | `docs/ARCHITECTURE.md` | Architecture | Boundary/runtime berubah | ADR required |
| ART-003 | Primitive catalog | `docs/PRIMITIVES.md` | Architecture/UI | Primitive baru | Reuse review |
| ART-004 | IPC contracts | `packages/contracts/schemas/*`, `data-contracts/*` | Platform | Command/event/schema berubah | Generation drift and contract tests |
| ART-005 | Context pack | `docs/CONTEXT_PACK.md` | Tech lead | Setiap task signifikan | Must update |
| ART-006 | Decision record | `artifacts/adr/*` | Author | Keputusan arsitektural | Approved status |
| ART-007 | Backlog | `docs/IMPLEMENTATION_PLAN.md` | Product/Lead | Planning iteration | Traceable IDs |
| ART-008 | Golden datasets | `tests/golden/*` | Analytics QA | Algorithm added | Expected values reviewed |
| ART-009 | Threat model | `artifacts/THREAT_MODEL.md` | Security | Trust boundary changes | Security review |
| ART-010 | Release manifest | `artifacts/releases/*` | Release | Release candidate | Checksums/SBOM |
| ART-011 | Data dictionary | `artifacts/DATA_DICTIONARY.md` | Data | Entity/schema changes | Migration linked |
| ART-012 | UX flows | `artifacts/UX_FLOWS.md` | UX | User flow changes | States complete |

## Required ADR format
```markdown
# ADR-NNN: Title
Status: Proposed | Accepted | Superseded
Date:
Context:
Decision:
Alternatives:
Consequences:
Migration/Rollback:
Related requirements/tasks:
```

## Accuracy evidence artifact
Setiap algoritma analitik wajib memiliki:
- definisi matematis/metode;
- null and edge-case policy;
- deterministic seed policy;
- golden input;
- expected output dan toleransi;
- independent cross-check method;
- benchmark note;
- known limitations.
