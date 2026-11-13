# Full access reset during first-run onboarding

Verified against OpenAI.Codex 26.1103.4239.0, `app/resources/app.asar`.

The onboarding completion handler (around byte 99978154) derives its settings
from the selected work mode:

```js
V = workMode !== "non_coding"
  ? { /* ... */ showFullAccessInComposer: true }
  : { /* ... */ showFullAccessInComposer: false };
// During the completion handler:
scope.set(agentModeAtom, "auto");
scope.set(fullAccessVisibilityAtom, V.showFullAccessInComposer);
```

The welcome role selection's Skip action explicitly selects
`roles: ["something_else"], workMode: "non_coding"`. The supplied target-machine
state has exactly those values, menu visibility false, local agent mode auto,
and the assistant's earlier permission-selection value still set to full-access.
This explains why a later manual provider configuration works and why fixed
2/15-second waits can complete before the user finishes onboarding.

The settings component (around byte 341300536) reads the visibility atom for
the displayed Full access switch. Its label says Full access, but its accessible
name and confirmation explain that it controls exposing Full access in the
composer. Actual selected agent permissions are separate.

The installer must wait for the genuine persisted
`electron-persisted-atom-state.last_completed_onboarding` marker, then reapply
the same provider configuration used by the provider editor, including selected
additional settings. It must not fabricate the marker or choose onboarding
answers for the user.

Regression checks simulate a user completing non-coding onboarding after 60
seconds, an already configured installation, a previously disabled installation,
cancellation, and a user who never completes onboarding. The Rust test also
checks that merely configuring permissions does not imply onboarding completion.

No target-machine GUI verification has been performed from this workspace.

## Visible option versus selected default (follow-up)

The later screenshot shows the option is visible but Request approvals is
selected. Onboarding alone does not explain this selection.

The legacy composer resolver at byte 188275931 uses a saved host choice only
when it differs from `config-derived-agent-mode-by-host-id[host]`. When both
values equal `full-access`, it ignores the saved choice. Its own selection
handler clears the config-derived host value to null before saving a choice.
Executing the actual expression extracted from the bundle reproduced both
cases: equal values yield undefined; a null cache yields `full-access`.

The writer now follows that selection handler for the local host only. All
full-access configuration paths also set `default_permissions` to
`:danger-full-access`, read by the newer permission-profile resolver at byte
148256252. Verification rejects stale host caches and missing profile defaults.
The Rust regression fixture preserves other hosts and verifies that a stale
cache fails validation and that applying the options repairs it.

The bundled Codex `codex.exe app-server` was also started with a temporary,
isolated CODEX_HOME. Its real `config/read` response accepted and returned
`default_permissions=":danger-full-access"` and `approval_policy="never"`.
The executable was copied to the build temp directory to execute it outside
WindowsApps; the running desktop instance and its home were not changed.

This validates persisted defaults and the extracted selection expression;
it is not a claim that the remote computer's GUI has been tested.

## Stop accepting a successful file check as UI confirmation

The next user test still showed Request approvals. That falsifies the claim
that the preceding file changes were sufficient. The one-click flow previously
read the same files it had just written immediately after starting Codex, so a
successful result never established that the live composer adopted them.

The flow now invokes `apply_codex_full_access` after onboarding and any required
reconfiguration. It connects only to the loopback debugging port assigned by
the assistant's launch, finds the real composer permission control, chooses
Full access using Codex's own menu/confirmation handlers, and returns success
only when the actual composer trigger shows Full access with its menu closed.
It does not edit labels or fabricate a successful UI state. Disabled options
produce a policy error; unavailable or incompatible controls time out with a
stage instead of reporting setup success. The native command is asynchronous.

Regression tests cover selection, confirmation, already-selected controls,
disabled permissions, absent composer, false-positive disk success, and the
HTTP/WebSocket path requiring renderer confirmation. They do not establish that
the remote user's installed build has been tested interactively.

## Best-effort mode (user feedback)

The confirmation failure text was removed at the user's request because it
confused end users. `apply_codex_full_access` now attempts the UI alignment and
returns success when the control is absent, disabled, or not confirmed within
20 seconds. The setup flow also swallows errors from this call, so a missing or
different composer never turns a completed installation into an error dialog.
