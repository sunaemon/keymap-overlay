resource "github_repository" "keymap_overlay" {
  name       = "keymap-overlay"
  visibility = "public"

  has_issues      = true
  has_projects    = false
  has_wiki        = false
  has_discussions = false

  allow_squash_merge  = true
  allow_merge_commit  = false
  allow_rebase_merge  = false
  allow_auto_merge    = false
  allow_update_branch = true

  delete_branch_on_merge      = true
  allow_forking               = true
  web_commit_signoff_required = false

  security_and_analysis {
    secret_scanning {
      status = "enabled"
    }

    secret_scanning_push_protection {
      status = "enabled"
    }
  }

  lifecycle {
    prevent_destroy = true
  }
}

resource "github_repository_vulnerability_alerts" "keymap_overlay" {
  repository = github_repository.keymap_overlay.name
  enabled    = true
}

resource "github_repository_dependabot_security_updates" "keymap_overlay" {
  repository = github_repository.keymap_overlay.name
  enabled    = true
}
