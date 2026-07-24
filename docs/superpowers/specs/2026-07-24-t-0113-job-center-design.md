# T-0113 Job Center UI Design

## Objective

Expose the durable background-job lifecycle in the active project dashboard without adding operation enqueue, analytics execution, automatic retry, or project migration.

## Scope

- bounded newest-first job listing through the T-0112 `JobClient`;
- live best-effort lifecycle updates reconciled by persistent job revision;
- manual refresh and bounded keyset pagination;
- cooperative cancellation with explicit confirmation and per-row pending state;
- loading, empty, initial error, non-blocking refresh error, event warning, schema-upgrade-required, and unavailable-runtime states;
- accessible status, progress, error, and action presentation;
- deterministic unit and render tests.

## Data and consistency model

`job_list` and `job_get` are authoritative. `job:lifecycle` is only a low-latency hint. A received snapshot replaces an existing row only when its revision is newer. Refresh replaces the first page with durable state. Pagination merges by job identity and retains the highest revision.

The UI requests 25 rows per page and retains at most 100 rows. These named limits keep UI memory bounded and match the T-0112 transport maximum. Filters apply only to the currently loaded bounded set and are labelled accordingly.

## Project compatibility

The Job Center is mounted only for an active project. Metadata schema 1 renders an explicit upgrade-required state and does not call job IPC or migrate data. Schema 2 uses the injected project-scoped job client. An unavailable native client renders a non-retriable desktop-runtime state.

## Cancellation

Only `QUEUED` and `RUNNING` rows offer cancellation. The user must confirm the selected job. While the command is pending, only that row is disabled and labelled. The command supplies the currently rendered persistent revision through the T-0112 client.

On cancellation failure, the UI attempts `job_get` to reconcile stale state. A safe typed error remains visible; the user can dismiss it or refresh. `CANCELLING` means requested, not stopped. Terminal states never expose cancellation.

## Accessibility and responsive behavior

Status is communicated with text and color. Known progress uses native progress semantics and explicit values; unknown totals remain textual. Desktop uses a semantic table, while narrow screens use labelled job cards. Alerts use polite live regions except the initial blocking error. The confirmation surface uses `alertdialog`, labelled controls, and deterministic focus placement.

## Non-goals

- operation enqueue or arbitrary execution;
- Python dispatch;
- retry orchestration;
- project schema upgrade;
- persistent UI preferences;
- global job totals beyond the bounded loaded set;
- mutation of source datasets or job metadata outside typed IPC.
