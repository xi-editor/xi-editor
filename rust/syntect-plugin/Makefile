# Makefile for installing syntect plugin on macOS and Linux

XDG_CONFIG_HOME ?= $(HOME)/.config
XI_CONFIG_DIR ?= $(XDG_CONFIG_HOME)/xi
XI_PLUGIN_DIR ?= $(XI_CONFIG_DIR)/plugins

target/release/xi-syntect-plugin:
	cargo build --release

install: manifest.toml target/release/xi-syntect-plugin
	install -d $(XI_PLUGIN_DIR)/syntect/bin
	install manifest.toml $(XI_PLUGIN_DIR)/syntect
	install target/release/xi-syntect-plugin $(XI_PLUGIN_DIR)/syntect/bin

.PHONY: install
