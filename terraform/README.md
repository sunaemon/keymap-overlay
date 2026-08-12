# GitHub Repository Settings

This directory manages the `sunaemon/keymap-overlay` repository settings.
Terraform uses the authenticated GitHub CLI session automatically: run
`gh auth login` once, then use the Make targets below. Do not set a token in a
Terraform file or commit local state.

## First Use

Install the development tools and authenticate GitHub CLI:

```bash
mise install
gh auth login
```

Initialize Terraform and inspect the existing settings:

```bash
make terraform-init
make terraform-plan
```

The configuration's import blocks adopt the existing GitHub resources on the
first `make terraform-apply`. The local `terraform.tfstate` is deliberately
ignored. Keep it on the machine used for administration; do not commit it
because state can contain sensitive provider data.

## Day-to-day Use

```bash
make terraform-format
make terraform-validate
make terraform-plan
make terraform-apply
```

The configuration manages repository-level merge settings and security
features. It does not yet manage branch protection: `main` has no branch
protection at the time this configuration was introduced, and required checks
need an explicit project policy before they are enforced.
