NAME       := kache
TEST_BINARY_ENV := KACHE_TEST_BINARY
TEST_TARGETS := --lib --tests
HOST       := $(shell rustc -vV | awk '/^host:/ {print $$2}')
TARGET     ?= $(subst -unknown-linux-gnu,-unknown-linux-musl,$(HOST))
TARGET_ENV := $(shell printf '%s' "$(TARGET)" | tr '[:lower:]-' '[:upper:]_')
CARGO_CMD  := $(if $(findstring -linux-musl,$(TARGET)),$(shell command -v musl-cargo 2>/dev/null || printf cargo),cargo)
cargo = $(if $(findstring -linux-musl,$(TARGET)),,$(if $(filter release-static test,$(1)),CARGO_TARGET_$(TARGET_ENV)_LINKER=clang ,)$(if $(filter release-static,$(1)),RUSTFLAGS="-Cpanic=abort")) MUSL_TARGET="$(TARGET)" MUSL_BUILD_MODE="$(1)" $(CARGO_CMD)

# Build directly by default. CI may opt into using a released kache as
# the wrapper; a dogfood RUSTC_WRAPPER anywhere else would wrap kache in
# itself (mutants baselines die that way), so it takes KACHE_SELF_HOST=1
# to keep it.
ifneq ($(KACHE_SELF_HOST),1)
unexport RUSTC_WRAPPER
endif

.PHONY: help build test check test-ci release verify-release lint \
  client-crates-check test-resource-guard-check pr mutants-diff \
  e2e bench bench-retry bench-trace bench-sccache bench-mbx bench-pr \
  bench-short test-bench-short bench-contention test-bench-contention \
  perf-gate-local trace-phases bump release-tag crates-status \
  crates-bootstrap crates-bootstrap-plan

build:
	$(call cargo,dev) build -p $(NAME) --bin kache --bin kache-engine

test:
	$(call cargo,test) test --quiet --workspace --lib --tests

test-ci:
	@test -x "target/$(TARGET)/dist/$(NAME)"
	$(TEST_BINARY_ENV)="$(CURDIR)/target/$(TARGET)/dist/$(NAME)" \
	$(call cargo,test) test --quiet --release $(TEST_TARGETS)

release:
	$(call cargo,release-static) clean -p $(NAME) --profile dist --target $(TARGET)
	$(call cargo,release-static) build --profile dist -p $(NAME) --bin kache --bin kache-engine \
	  -Z build-std=std \
	  -Z build-std-features= \
	  --target $(TARGET)

verify-release:
	@test -f "target/$(TARGET)/dist/$(NAME)"
	@if echo "$(TARGET)" | grep -q -- '-linux-musl$$'; then \
		command -v readelf >/dev/null || { echo 'readelf is required for release verification'; exit 1; }; \
		file "target/$(TARGET)/dist/$(NAME)" | grep -Eq 'static-pie linked|statically linked' || { echo 'release is not statically linked'; exit 1; }; \
		file "target/$(TARGET)/dist/$(NAME)" | grep -q 'stripped' || { echo 'release is not stripped'; exit 1; }; \
		! readelf -l "target/$(TARGET)/dist/$(NAME)" | grep -q INTERP || { echo 'release has a dynamic ELF interpreter'; exit 1; }; \
		! readelf -d "target/$(TARGET)/dist/$(NAME)" | grep -q NEEDED || { echo 'release has dynamic dependencies'; exit 1; }; \
	else \
		echo "Skipping ELF checks for $(TARGET)"; \
	fi

lint:
	cargo clippy --workspace --all-targets -- -D warnings

help:
	@echo "build                    build the dev binaries"
	@echo "test                     run the workspace tests"
	@echo "check                    the pre-PR gate: lint + client crates + resource guard + tests"
	@echo "pr [BASE=origin/main]    check plus changed-lines mutation testing against BASE"
	@echo "e2e                      fixture scenarios cold/warm/noop through the e2e harness"
	@echo "bench [PROFILE=] [ARGS=] list bench profiles, or cold/warm a profile (ARGS passes through)"
	@echo "bench-retry/-trace/-sccache/-mbx  bench variants; PROFILE selects, ARGS passes through"
	@echo "bench-pr                 one Kache side of hk, eza and aube"
	@echo "bench-short PROJECT= [SAMPLES=6] [ARGS=]  isolated builds, contention optional"
	@echo "bench-contention [ARGS=] six overlapping Cargo jobs on Linux"
	@echo "perf-gate-local [ARGS=]  HEAD against its merge base with the gate verdict"
	@echo "trace-phases DIR= [ARGS=] roll up phase traces"
	@echo "bump VERSION=             bump the workspace version everywhere"
	@echo "release-tag              cut the release tag for the merged manifest version"
	@echo "crates-status/-bootstrap/-bootstrap-plan  crates.io publish helpers"

# What a PR must survive before opening: the clippy gate, the client-only
# dependency graph (feature unification must not hide a dependency), the
# resource-guard self-check, and the full workspace tests. Changed Rust
# lines additionally go through `make pr` (same mutants-diff scope CI runs).
check: lint client-crates-check test-resource-guard-check test

# Check the client-only dependency graph separately so feature unification
# cannot hide a dependency.
client-crates-check:
	cargo check --locked -p kache-client -p kache-config -p kache-protocol -p kache-ipc
	cargo test --locked -p kache-ipc

# Verify both the raised-limit and bounded-thread fallback paths without
# changing the invoking shell's resource limits.
test-resource-guard-check:
	./scripts/test-with-test-resources.sh

# Same changed-line mutants job CI runs. Docs-only diffs skip mutants.
# BASE is the merge base, default `origin/main`.
BASE ?= origin/main
pr: check
	mkdir -p tmp/mutants
	python3 scripts/mutation-diff.py "$$(git merge-base "$(BASE)" HEAD)" HEAD > tmp/mutants/pr.diff
	if [ ! -s tmp/mutants/pr.diff ]; then \
	  echo "no Rust diff against $(BASE); skipping mutants-diff"; \
	else \
	  $(MAKE) mutants-diff DIFF=tmp/mutants/pr.diff; \
	fi

# Mutation-test changed Rust lines. DIFF must describe the current working
# tree. Install the pinned tool with:
# cargo install --locked cargo-mutants --version 27.1.0
mutants-diff:
	$(if $(DIFF),,$(error DIFF is required))
	./scripts/with-test-resources.sh cargo mutants --workspace --all-features --in-diff "$(DIFF)" --exclude 'crates/kache-core/**' --exclude 'crates/kache-proofs/**' --baseline run --timeout 300 --build-timeout 600 --output tmp/mutants/diff $(ARGS)

# Run the end-to-end harness against every e2e scenario in scenarios/.
# Builds kache + harness in release mode, drives each fixture through
# cold → warm → noop. Writes tmp/e2e/results.json.
e2e:
	cargo build --release -p kache
	cargo build --release -p kache-e2e
	./target/release/kache-scenario \
	  --kache ./target/release/kache \
	  --scenarios ./scenarios \
	  --select suite:e2e \
	  --select tier:gate \
	  --out tmp/e2e/results.json

# Bench a scenario twice against one shared kache cache — cold, then warm —
# and report wall-clock, speedup, hit rate and a verdict. Omit PROFILE to
# list profiles; PROFILE is a name filter (firefox matches bench-firefox).
# Extra flags pass through ARGS, e.g. ARGS="--skip-clone".
PROFILE ?=
ARGS ?=
bench:
	@if [ -z "$(PROFILE)" ]; then \
	  cargo build -q --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --list --select suite:bench --select backend:kache; \
	else \
	  cargo build --release -p kache; \
	  cargo build --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --kache ./target/release/kache --select suite:bench --select backend:kache --profile "$(PROFILE)" $(ARGS); \
	fi

# Retry the warm phase only from the previous run's cold-state snapshot.
bench-retry:
	@if [ -z "$(PROFILE)" ]; then \
	  cargo build -q --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --list --select suite:bench --select backend:kache; \
	else \
	  cargo build --release -p kache; \
	  cargo build --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --kache ./target/release/kache --select suite:bench --select backend:kache --profile "$(PROFILE)" --retry $(ARGS); \
	fi

# Full bench with cache-key tracing in both phases, then a per-crate
# cold/warm key-divergence diff.
bench-trace:
	@if [ -z "$(PROFILE)" ]; then \
	  cargo build -q --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --list --select suite:bench --select backend:kache; \
	else \
	  cargo build --release -p kache; \
	  cargo build --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --kache ./target/release/kache --select suite:bench --select backend:kache --profile "$(PROFILE)" --trace-keys $(ARGS); \
	fi

# Same cold/warm clone benchmark with sccache as the compiler cache.
bench-sccache:
	@if [ -z "$(PROFILE)" ]; then \
	  cargo build -q --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --list --cache-backend sccache --select suite:bench --select backend:sccache; \
	else \
	  cargo build --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --cache-backend sccache --select suite:bench --select backend:sccache --profile "$(PROFILE)" $(ARGS); \
	fi

# Same shape through mbx (`mbx` on PATH).
bench-mbx:
	@if [ -z "$(PROFILE)" ]; then \
	  cargo build -q --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --list --cache-backend mbx --select suite:bench --select backend:mbx; \
	else \
	  cargo build --release -p kache-e2e --bin kache-scenario; \
	  ./target/release/kache-scenario --cache-backend mbx --select suite:bench --select backend:mbx --profile "$(PROFILE)" $(ARGS); \
	fi

# One Kache side of the PR subjects; use a separate checkout for each version.
bench-pr:
	$(MAKE) bench PROFILE=hk ARGS="--warm-same-tree"
	$(MAKE) bench PROFILE=eza ARGS="--warm-same-tree"
	$(MAKE) bench PROFILE=aube ARGS="--warm-same-tree"

# Run isolated builds (2 cold / 6 warm) and contention (2 cold / 6 warm).
# Add --mbx/--sccache via ARGS to measure those tools as well.
SAMPLES ?= 6
bench-short:
	$(if $(PROJECT),,$(error PROJECT is required))
	cargo build --release -p kache -p kache-e2e --bin kache --bin kache-engine --bin kache-scenario
	python3 scripts/bench-short.py --project "$(PROJECT)" --samples "$(SAMPLES)" \
	  --engine target/release/kache-scenario --kache target/release/kache \
	  --output "tmp/bench/bench-$(PROJECT)" $(ARGS)

# Validate sample isolation, result admission and paired comparisons.
test-bench-short:
	@python3 -m unittest discover -s scripts/bench/tests -t scripts

# Run six overlapping Cargo jobs on Linux; pass --output with a fresh directory.
bench-contention:
	cargo build --release -p kache
	python3 scripts/bench-contention.py --arm kache=./target/release/kache,1 $(ARGS)

test-bench-contention:
	python3 scripts/test-bench-contention.py

# Measure HEAD against its merge base the way the perf gate does, without a PR.
perf-gate-local:
	python3 scripts/perf-gate-local.py $(ARGS)

# Roll up a build's phase traces: self time per phase, summed over processes.
trace-phases:
	$(if $(DIR),,$(error DIR is required))
	python3 scripts/trace-phases.py "$(DIR)" $(ARGS)

# Bump the workspace version everywhere in one shot via `cargo set-version`
# (NOT a broad `cargo update`, so pinned kunobi-* git deps stay valid).
# Use a DOTTED-numeric prerelease (`-rc.4`, not `-rc4`): crates.io is
# permanent and semver orders the no-dot form lexically.
bump:
	$(if $(VERSION),,$(error VERSION is required))
	set -euo pipefail; \
	case "$(VERSION)" in \
	  *-rc[0-9]*|*-alpha[0-9]*|*-beta[0-9]*) \
	    echo "use a dotted prerelease (e.g. 0.5.0-rc.4), not the no-dot form" >&2; \
	    exit 1 ;; \
	esac; \
	if ! command -v cargo-set-version >/dev/null 2>&1; then \
	  echo "cargo-edit (cargo set-version) not found — installing it…"; \
	  if command -v cargo-binstall >/dev/null 2>&1; then \
	    cargo binstall -y cargo-edit; \
	  else \
	    cargo install cargo-edit; \
	  fi; \
	fi; \
	cargo set-version --workspace $(VERSION); \
	cargo check --workspace; \
	./scripts/check-version-consistency.sh; \
	echo "Bumped to $(VERSION). Commit + open a PR; after merge, cut the tag with 'make release-tag'."

# Cut the release tag for the merged manifest version. Refuses unless the
# tree is releasable (clean, on `main`, in sync with origin/main).
release-tag:
	set -euo pipefail; \
	[ -z "$$(git status --porcelain)" ] || { echo "working tree is dirty — commit or stash first" >&2; exit 1; }; \
	branch="$$(git rev-parse --abbrev-ref HEAD)"; \
	[ "$$branch" = "main" ] || { echo "not on main (on '$$branch') — releases are cut from main" >&2; exit 1; }; \
	git fetch --quiet origin main; \
	[ "$$(git rev-parse HEAD)" = "$$(git rev-parse origin/main)" ] || { echo "local main is not in sync with origin/main — pull/push first" >&2; exit 1; }; \
	version="$$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="kache"))')"; \
	tag="v$${version}"; \
	./scripts/check-version-consistency.sh "$$tag"; \
	git rev-parse -q --verify "refs/tags/$$tag" >/dev/null && { echo "tag $$tag already exists" >&2; exit 1; } || true; \
	git tag -a "$$tag" -m "$$tag"; \
	git push origin "$$tag"; \
	echo "pushed $$tag — the gated release pipeline will run; watch CI."

# Compare publishable workspace crates to crates.io.
crates-status:
	python3 scripts/crates-io.py status $(ARGS)

# One-time first publish, then Trusted Publishing setup. Preview with
# `make crates-bootstrap-plan`.
crates-bootstrap:
	python3 scripts/crates-io.py bootstrap $(ARGS)

crates-bootstrap-plan:
	python3 scripts/crates-io.py bootstrap --dry-run
