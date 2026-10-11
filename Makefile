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

## release: optimized build of translator and translator-tui in target/release, with native libs linked next to them
.PHONY: release
release: native-libs
	$(CARGO) build --release -p translator-ui -p translator-tui
	target_dir=$$($(CARGO) metadata --format-version 1 --no-deps \
		| sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'); \
	test -n "$$target_dir" || { echo "cargo metadata failed" >&2; exit 1; }; \
	ln -sfn $(NATIVE_LIB_DIR) "$$target_dir/release/lib" && \
	echo "built $$target_dir/release/translator and $$target_dir/release/translator-tui"

## native-libs: download libmoonshine / liblitert-lm into native/<arch>/lib (fetched again when scripts/fetch-native-libs.sh changes; make -B native-libs forces it)
.PHONY: native-libs
native-libs: $(NATIVE_LIB_DIR)/.complete

# Depends on the script, so pinning another wheel version fetches again.
$(NATIVE_LIB_DIR)/.complete: scripts/fetch-native-libs.sh
	scripts/fetch-native-libs.sh $(NATIVE_ARCH)

MODEL_DIR ?= $(CURDIR)/models

# The config whose [tts_voices] translator-models checks and fetches; empty
# means the translator's default (~/.config/gemma-translator/config.toml).
CONFIG ?=
MODELS_ARGS := --lib-dir $(NATIVE_LIB_DIR) --model-dir $(MODEL_DIR) $(if $(CONFIG),--config $(CONFIG))

## models: download every model into MODEL_DIR (default ./models, ~3.7 GB; writes MODEL_DIR/.complete when done), plus the voices config.toml names (CONFIG=<file>)
.PHONY: models
models: native-libs
	mkdir -p $(MODEL_DIR)
	[ "$(MODEL_DIR)" != "$(CURDIR)/models" ] || printf '*\n' > $(MODEL_DIR)/.gitignore
	$(CARGO) run --release -p translator-models -- $(MODELS_ARGS)
	touch $(MODEL_DIR)/.complete

FFI_ENV := TRANSLATOR_NATIVE_LIB_DIR=$(NATIVE_LIB_DIR) TRANSLATOR_MODEL_DIR=$(MODEL_DIR)

## voices: list every TTS voice per language (* = downloaded), for [tts_voices] in config.toml
.PHONY: voices
voices: native-libs
	$(CARGO) run --release -p translator-models -- $(MODELS_ARGS) --list-voices

## test-ffi: run the tests that need native libraries and models (after make models)
.PHONY: test-ffi
test-ffi:
	$(FFI_ENV) $(CARGO) test --workspace -- --ignored --skip soak --test-threads=1

## eval-stt: score speech recognition on the recorded corpus in eval/ (EVAL_ARGS="--runs 3 --only clean")
.PHONY: eval-stt
eval-stt:
	$(FFI_ENV) $(CARGO) run --release -q -p translator-io --example stt_eval -- $(EVAL_ARGS)

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

## clean: remove build output (target/, dist/); keeps native/ and models/
.PHONY: clean
clean:
	$(CARGO) clean
	rm -rf dist
