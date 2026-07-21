# Project Lifecycle UX Flows

## Scope

T-0101 connects the desktop shell to the transactional project core. It does not import data, execute analytics, repair corrupted projects, or persist a recent-project list.

## Create project

1. Empty dashboard exposes **Buat proyek** and **Buka proyek**.
2. **Buat proyek** opens an accessible dialog with a required project name.
3. Submit opens the operating-system save dialog with a sanitized `.teratai` suggestion.
4. Cancelling either dialog returns to the empty dashboard without a write.
5. Confirmation enters a creating state and disables conflicting actions.
6. Successful native validation activates the project dashboard.
7. A typed failure closes the form and displays the matching permission, recovery, or general error state.

## Open project

1. **Buka proyek** opens the operating-system directory picker.
2. Cancelling returns to the current state without mutation.
3. Confirmation enters an opening state while Rust performs read-only validation.
4. A valid descriptor activates the project dashboard.
5. Permission denial presents a path-selection remediation.
6. Recovery/corruption presents non-destructive guidance; no delete, overwrite, or automatic repair action is offered.

## Close project

1. **Tutup proyek** enters a closing state.
2. The native in-memory session is cleared.
3. The UI returns to the empty dashboard. Project files remain unchanged.

## State coverage

| State | User-visible behavior | Allowed recovery |
|---|---|---|
| Loading | startup status while checking the native session | wait for completion |
| Empty | create/open actions enabled | choose create or open |
| Selecting/creating/opening/closing | progress label and conflicting actions disabled | cancel only while the system picker is open |
| Permission | explains that a desktop/system-approved location is required | choose another approved location or run the native desktop app |
| Recovery/corruption | explains that the project needs inspection and preserves user data | select another project; manual recovery is intentionally out of scope |
| Error | safe localized message and correlation ID | return to project selection |
| Active | verified project identity, path, schema, and localized creation time | close the in-memory session |
