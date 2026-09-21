# Coverage Regression Policy

Codecov reports coverage from three required CI uploads on every pull request:

| Upload    | Contents                                   |
| --------- | ------------------------------------------ |
| `linux`   | Python plus shared and Linux Rust coverage |
| `macos`   | macOS-only Rust coverage                   |
| `windows` | Windows-only Rust coverage                 |

The merge-blocking status is `codecov/patch`. It requires at least 80% coverage
of changed executable lines, with a one-percentage-point tolerance, so it fails
below 79%. The tolerance accommodates platform-specific compilation paths while
still requiring tests to exercise the substantial majority of new behavior.
Documentation, workflow configuration, generated files, and uninstrumented
platform code do not add executable lines to a patch result. The policy does
not justify tests that only duplicate implementation details; tests should
exercise observable behavior, error handling, or a regression's trigger.

`codecov/project` remains informational during this rollout. The current merged
baseline, measured from the three reports for `01b42c47` on 2026-09-21, is
74.59%. The preceding two merged commits measured 74.60%. After the policy has
seen several ordinary feature changes without report instability, set an
explicit project target from the then-current merged baseline and make that
status required in the repository ruleset.

Codecov waits for all three uploads (`after_n_builds: 3`) and for CI completion
before publishing either status. If an upload is missing or delayed,
`codecov/patch` remains pending instead of evaluating a partial report. A
failed uploader also fails its required platform job. The active `maind`
ruleset requires `codecov/patch`, in addition to the five native platform
checks, so a missing, failing, or below-threshold patch status blocks merging.

To validate a policy edit, post `codecov.yml` to `https://api.codecov.io/validate`.
For behavior checks, use a pull request with a covered changed line to confirm a
pass, then a temporary pull request with enough uncovered executable changed
lines to fall below 79% and confirm `codecov/patch` fails. Do not merge the
temporary regression.
