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

## native-libs: download libmoonshine / liblitert-lm for this machine into native/<arch>/lib
.PHONY: native-libs
native-libs:
	scripts/fetch-native-libs.sh $(NATIVE_ARCH)

MODEL_DIR ?= $(CURDIR)/models

## models: download every model into MODEL_DIR (default ./models, ~3.5 GB)
.PHONY: models
models: native-libs
	mkdir -p $(MODEL_DIR)
	[ "$(MODEL_DIR)" != "$(CURDIR)/models" ] || printf '*\n' > $(MODEL_DIR)/.gitignore
	$(CARGO) run --release -p translator-models -- --lib-dir $(NATIVE_LIB_DIR) --model-dir $(MODEL_DIR)

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
