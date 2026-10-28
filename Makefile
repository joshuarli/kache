NAME       := kache
TEST_BINARY_ENV := KACHE_TEST_BINARY
TEST_TARGETS := --lib --tests
HOST       := $(shell rustc -vV | awk '/^host:/ {print $$2}')
TARGET     ?= $(subst -unknown-linux-gnu,-unknown-linux-musl,$(HOST))
TARGET_ENV := $(shell printf '%s' "$(TARGET)" | tr '[:lower:]-' '[:upper:]_')
CARGO_CMD  := $(if $(findstring -linux-musl,$(TARGET)),$(shell command -v musl-cargo 2>/dev/null || printf cargo),cargo)
cargo = $(if $(findstring -linux-musl,$(TARGET)),,$(if $(filter release-static test,$(1)),CARGO_TARGET_$(TARGET_ENV)_LINKER=clang ,)$(if $(filter release-static,$(1)),RUSTFLAGS="-Cpanic=abort")) MUSL_TARGET="$(TARGET)" MUSL_BUILD_MODE="$(1)" $(CARGO_CMD)

.PHONY: build test check test-ci release verify-release lint

build:
	$(call cargo,dev) build -p $(NAME) --bin kache --bin kache-engine

test:
	$(call cargo,test) test --quiet --workspace --lib --tests

check:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace --lib --tests

test-ci:
	@test -x "target/$(TARGET)/dist/$(NAME)"
	$(TEST_BINARY_ENV)="$(CURDIR)/target/$(TARGET)/dist/$(NAME)" \
	$(call cargo,test) test --quiet --release --features remote $(TEST_TARGETS)

# Prereleases ship remote backends; the default build stays local-only.
release:
	$(call cargo,release-static) clean -p $(NAME) --profile dist --target $(TARGET)
	$(call cargo,release-static) build --profile dist -p $(NAME) --bin kache --bin kache-engine --features remote \
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
