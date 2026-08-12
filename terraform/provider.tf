# With no explicit token, the GitHub provider falls back to the authenticated
# GitHub CLI session (`gh auth token`). This keeps credentials out of this
# repository, Terraform state, shell history, and Makefile recipes.
provider "github" {}
