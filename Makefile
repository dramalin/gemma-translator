# Gemma Translator (Rust UI) — build orchestration.
# `make run-mock` works on any Linux or macOS desktop; `package-pi` builds the Raspberry Pi package.

CARGO     := cargo
NATIVE_ARCH    := $(shell uname -m)
NATIVE_LIB_DIR := $(CURDIR)/native/$(NATIVE_ARCH)/lib

.DEFAULT_GOAL := help

## help: list available targets
.PHONY: help
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'

## setup: check system tools and fetch everything the project downloads (SETUP_ARGS=--yes skips the model prompt)
.PHONY: setup
setup:
	scripts/setup-env.sh $(SETUP_ARGS)

## test: run the Rust tests
.PHONY: test
test:
	$(CARGO) test --workspace

## run-mock: run the UI on the desktop with mock speech and translation
.PHONY: run-mock
run-mock:
	$(CARGO) run -p translator-ui -- --mock

# Builds package $(1), links native/<arch>/lib next to its debug executable
# (the native libraries load from <exe dir>/lib) and runs binary $(2) on the
# models in MODEL_DIR.
define run_native
	$(CARGO) build -p $(1)
	target_dir=$$($(CARGO) metadata --format-version 1 --no-deps \
		| sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'); \
	test -n "$$target_dir" || { echo "cargo metadata failed" >&2; exit 1; }; \
	ln -sfn $(NATIVE_LIB_DIR) "$$target_dir/debug/lib" && \
	"$$target_dir/debug/$(2)" --model-dir $(MODEL_DIR) $(ARGS)
endef

## run: run the Slint UI with in-process inference on MODEL_DIR (default ./models)
.PHONY: run
run: native-libs
	$(call run_native,translator-ui,translator)

## run-tui: run the terminal UI with in-process inference (ARGS=--mock for mock services)
.PHONY: run-tui
run-tui: native-libs
	$(call run_native,translator-tui,translator-tui)

## native-libs: download libmoonshine / liblitert-lm into native/<arch>/lib (skipped when present; make -B native-libs refetches)
.PHONY: native-libs
native-libs: $(NATIVE_LIB_DIR)/.complete

$(NATIVE_LIB_DIR)/.complete:
	scripts/fetch-native-libs.sh $(NATIVE_ARCH)

MODEL_DIR ?= $(CURDIR)/models

## models: download every model into MODEL_DIR (default ./models, ~3.8 GB; writes MODEL_DIR/.complete when done)
.PHONY: models
models: native-libs
	mkdir -p $(MODEL_DIR)
	[ "$(MODEL_DIR)" != "$(CURDIR)/models" ] || printf '*\n' > $(MODEL_DIR)/.gitignore
	$(CARGO) run --release -p translator-models -- --lib-dir $(NATIVE_LIB_DIR) --model-dir $(MODEL_DIR)
	touch $(MODEL_DIR)/.complete

FFI_ENV := TRANSLATOR_NATIVE_LIB_DIR=$(NATIVE_LIB_DIR) TRANSLATOR_MODEL_DIR=$(MODEL_DIR)

## test-ffi: run the tests that need native libraries and models (after make models)
.PHONY: test-ffi
test-ffi:
	$(FFI_ENV) $(CARGO) test --workspace -- --ignored --skip soak --test-threads=1

## test-soak: long-running native memory tests
.PHONY: test-soak
test-soak:
	$(FFI_ENV) $(CARGO) test --release --workspace -- --ignored soak --test-threads=1 --nocapture

## fmt: format all Rust code
.PHONY: fmt
fmt:
	$(CARGO) fmt --all

## clippy: lint with warnings as errors
.PHONY: clippy
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

## package-pi: build dist/gemma-translator-pi.tar for the Raspberry Pi 5 (x86_64 Linux; needs cross 0.2.5 and Docker)
.PHONY: package-pi
package-pi:
	MODEL_DIR=$(MODEL_DIR) scripts/package-pi.sh
