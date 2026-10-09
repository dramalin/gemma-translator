# Gemma Translator (Rust UI) — build orchestration.
# `make run-mock` works on any Linux desktop; `build-pi` targets the Raspberry Pi.

CARGO     := cargo
PI_TARGET := aarch64-unknown-linux-gnu
NATIVE_ARCH    := $(shell uname -m)
NATIVE_LIB_DIR := $(CURDIR)/native/$(NATIVE_ARCH)/lib

.DEFAULT_GOAL := help

## help: list available targets
.PHONY: help
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'

## test: run Rust and Python unit tests
.PHONY: test
test:
	$(CARGO) test --workspace
	python3 -m unittest discover -s backend -p 'test_*.py'

## run-mock: run the UI on the desktop with mock speech and translation
.PHONY: run-mock
run-mock:
	$(CARGO) run -p translator-ui -- --mock

## run: run the UI against the local speech service and litert-lm
.PHONY: run
run:
	$(CARGO) run -p translator-ui

## run-native: run the UI with in-process inference (config: inference_backend = "native", model_dir = "<repo>/models")
.PHONY: run-native
run-native: native-libs
	$(CARGO) build -p translator-ui
	target_dir=$$($(CARGO) metadata --format-version 1 --no-deps \
		| sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'); \
	ln -sfn $(NATIVE_LIB_DIR) "$$target_dir/debug/lib" && "$$target_dir/debug/translator"

## native-libs: download libmoonshine / liblitert-lm into native/<arch>/lib (skipped when present; make -B native-libs refetches)
.PHONY: native-libs
native-libs: $(NATIVE_LIB_DIR)/liblitert-lm.so

$(NATIVE_LIB_DIR)/liblitert-lm.so:
	scripts/fetch-native-libs.sh $(NATIVE_ARCH)

MODEL_DIR ?= $(CURDIR)/models

## models: download every model into MODEL_DIR (default ./models, ~3.8 GB)
.PHONY: models
models: native-libs
	mkdir -p $(MODEL_DIR)
	[ "$(MODEL_DIR)" != "$(CURDIR)/models" ] || printf '*\n' > $(MODEL_DIR)/.gitignore
	$(CARGO) run --release -p translator-models -- --lib-dir $(NATIVE_LIB_DIR) --model-dir $(MODEL_DIR)

FFI_ENV := TRANSLATOR_NATIVE_LIB_DIR=$(NATIVE_LIB_DIR) TRANSLATOR_MODEL_DIR=$(MODEL_DIR)

## test-ffi: run the tests that need native libraries and models (after make models)
.PHONY: test-ffi
test-ffi:
	$(FFI_ENV) $(CARGO) test --workspace -- --ignored --skip soak --test-threads=1

## test-soak: long-running native memory tests
.PHONY: test-soak
test-soak:
	$(FFI_ENV) $(CARGO) test --release --workspace -- --ignored soak --test-threads=1

## fmt: format all Rust code
.PHONY: fmt
fmt:
	$(CARGO) fmt --all

## clippy: lint with warnings as errors
.PHONY: clippy
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

## build-pi: release build for the Pi with the KMS backend (needs the aarch64 toolchain)
.PHONY: build-pi
build-pi:
	$(CARGO) build --release -p translator-ui --no-default-features --features kms --target $(PI_TARGET)
