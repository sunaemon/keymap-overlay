SHELL := /bin/bash

# Public entry point. Implementation stays non-recursive and is grouped by
# concern so every existing `make <target>` command remains stable.
include mk/config.mk
include mk/development.mk
include mk/verification.mk
include mk/release.mk
include mk/overlay.mk
include mk/install.mk
include mk/firmware.mk
