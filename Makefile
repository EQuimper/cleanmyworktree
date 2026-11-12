# Common tasks. `make` lists them.
#
# Releasing: `make release-patch` (or -minor / -major, or `make release VERSION=1.2.3`).
# Every release target checks the repo, runs fmt/clippy/tests and `dist plan`, then commits the
# version bump, tags vX.Y.Z and pushes both; the tag starts the Release workflow (binaries,
# GitHub release, Homebrew formula). Append DRY_RUN=1 to check everything without changing anything.

.DEFAULT_GOAL := help
.PHONY: help check fmt test install release release-patch release-minor release-major

RELEASE := scripts/release.sh
DRY := $(if $(DRY_RUN),--dry-run,)

help: ## List targets
	@grep -hE '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

check: ## fmt, clippy, tests (what CI and releases require)
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

fmt: ## Format the code
	cargo fmt

test: ## Run the tests
	cargo test

install: ## Install this checkout as `cmw` (~/.cargo/bin)
	cargo install --path . --locked

release-patch: ## Release X.Y.Z+1   (DRY_RUN=1 to check only)
	@$(RELEASE) patch $(DRY)

release-minor: ## Release X.Y+1.0   (DRY_RUN=1 to check only)
	@$(RELEASE) minor $(DRY)

release-major: ## Release X+1.0.0   (DRY_RUN=1 to check only)
	@$(RELEASE) major $(DRY)

release: ## Release VERSION=X.Y.Z (DRY_RUN=1 to check only)
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=X.Y.Z [DRY_RUN=1]"; exit 2; }
	@$(RELEASE) $(VERSION) $(DRY)
