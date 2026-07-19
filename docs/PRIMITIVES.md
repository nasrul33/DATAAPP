# Primitive System

Primitive adalah unit paling kecil yang stabil, dapat diuji, dan dapat disusun ulang. Codex wajib memakai primitive sebelum membuat implementasi ad-hoc.

## A. UI primitives
- `Button`, `IconButton`, `Input`, `Select`, `Checkbox`, `Switch`.
- `Dialog`, `Drawer`, `Popover`, `Tooltip`, `CommandMenu`.
- `DataGrid`, `VirtualList`, `Pagination`, `ColumnFilter`.
- `Badge`, `StatusIndicator`, `ProgressBar`, `Skeleton`.
- `EmptyState`, `ErrorState`, `PermissionState`.
- `MetricCard`, `ChartFrame`, `InspectorPanel`, `SplitPane`.

Rules:
- Accessible keyboard navigation.
- Loading/disabled/error semantics konsisten.
- Tidak ada warna status tanpa label/ikon.
- Tidak membuat primitive baru bila komposisi primitive lama cukup.

## B. Data primitives
- `ScalarType`: string, integer, float, decimal, boolean, date, datetime, categorical.
- `ColumnSchema`.
- `DatasetRef` dan `DatasetVersionRef`.
- `RowLocator`: stable row identity/source lineage.
- `DataPreview`: schema + bounded rows + pagination token.
- `ProfileMetric`: metric name, value, population, method.
- `DataFingerprint`: content/schema hash.

## C. Operation primitives
Semua operasi mengikuti kontrak:
```text
OperationSpec
- operation_type
- input_dataset_refs[]
- canonical_parameters
- expected_output_schema?
- deterministic_seed?

OperationRun
- operation_id
- status
- engine_version
- started_at/finished_at
- input_fingerprints[]
- output_dataset_refs[]
- metrics
- warnings[]
- error?
```

Kategori:
- Source: import/read.
- Transform: select/filter/cast/derive/clean/join/aggregate.
- Analyze: profile/statistics/correlation.
- Detect: duplicate/outlier/rule/Benford.
- Visualize: chart spec from stable dataset.
- Sink: export/materialize/report.

## D. Workflow primitives
- `NodeDefinition`: ports, parameter schema, executor type.
- `NodeInstance`: definition + configured parameters.
- `Port`: typed input/output.
- `Edge`: source port to target port.
- `GraphValidationResult`.
- `ExecutionPlan`: topological order + cache strategy.
- `NodeRunResult`.

Rules:
- DAG only for MVP.
- Type compatibility checked before run.
- Invalid node prevents execution with actionable message.
- Re-run only invalidated downstream nodes.

## E. Analytics primitives
- `PopulationDefinition`: dataset/filter/group used as comparison basis.
- `DetectorSpec`: algorithm, features, threshold, null policy, seed.
- `DetectorResult`: row locator, score, severity, reason codes, evidence metrics.
- `Explanation`: human-readable summary + machine-readable factors.
- `ValidationMetric`: precision/recall/F1 or numerical tolerance as applicable.

## F. Audit primitives
- `RiskLevel`: LOW, MEDIUM, HIGH, CRITICAL.
- `ReviewStatus`: DETECTED, UNDER_REVIEW, CONFIRMED_ANOMALY, FALSE_POSITIVE, FINDING.
- `Finding`: condition, criteria, cause, effect, recommendation.
- `EvidenceReference`: dataset version, row locator, column values snapshot/hash, attachment.
- `AuditEvent`: actor, action, target, before/after hash, timestamp, correlation ID.

## G. Job primitives
- `JobStatus`: QUEUED, RUNNING, SUCCEEDED, FAILED, CANCELLING, CANCELLED.
- `JobProgress`: current, total, unit, phase, message.
- `CancellationToken`.
- `RetryPolicy`: only explicitly safe/idempotent jobs.
- `ResourceEstimate`: expected memory, disk, duration class.
