# Import blocks let a new administrator adopt the existing GitHub resources
# through the normal plan/apply workflow. Once imported, they are no-ops.
import {
  to = github_repository.keymap_overlay
  id = "keymap-overlay"
}

import {
  to = github_repository_vulnerability_alerts.keymap_overlay
  id = "keymap-overlay"
}

import {
  to = github_repository_dependabot_security_updates.keymap_overlay
  id = "keymap-overlay"
}
