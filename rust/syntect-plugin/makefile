XDG_CONFIG_HOME ?= $(HOME)/.config
XI_CONFIG_DIR ?= $(XDG_CONFIG_HOME)/xi
XI_PLUGIN_DIR ?= $(XI_CONFIG_DIR)/plugins

target/release/xi-syntect-plugin:
	cargo build --release

install: manifest.toml target/release/xi-syntect-plugin
	install -D manifest.toml $(XI_PLUGIN_DIR)/syntect/manifest.toml
	install -D target/release/xi-syntect-plugin $(XI_PLUGIN_DIR)/syntect/bin/xi-syntect-plugin

.PHONY: install
