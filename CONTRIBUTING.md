# Contributing

Thanks for helping improve keymap-overlay. The project is currently beta, so
small, focused pull requests with clear manual verification notes are especially
valuable.

## Before Starting

Read [AGENTS.md](AGENTS.md) for the repository architecture, toolchain, coding
standards, and platform-specific verification requirements. The authoritative
system design is [doc/design.md](doc/design.md).

For a substantial behavioral change, open an issue first so the expected user
workflow and cross-platform impact can be agreed before implementation.

## Verification

Run the checks relevant to the files changed:

```bash
make format
make lint
make test
make test-rust
make test-installer-sh
make build-overlay
make audit
```

`make format` must leave no diff. Platform UI and login-service changes also
need the manual checks described in AGENTS.md because CI has no interactive
desktop or attached keyboard.

## Pull Requests

Explain the user-visible change, list the commands and platforms tested, and
call out anything that could not be verified. Keep generated files, including
`Cargo.lock` and `THIRD-PARTY-LICENSES.html`, in sync with their sources.

By contributing, you agree that your contribution is licensed under this
repository's applicable licenses.
