# Makefile for installing syntect plugin on macOS and Linux

XDG_CONFIG_HOME ?= $(HOME)/.config
XI_CONFIG_DIR ?= $(XDG_CONFIG_HOME)/xi
XI_PLUGIN_DIR ?= $(XI_CONFIG_DIR)/plugins

lang_configs = assets/*.toml

out/syntect: $(lang_configs) target/release/xi-syntect-plugin
	mkdir -p out/syntect/bin
	cp $(lang_configs) out/syntect
	cp target/release/xi-syntect-plugin out/syntect/bin
	cp manifest.toml out/syntect/manifest.toml

target/release/xi-syntect-plugin: src/*.rs Cargo.toml Cargo.lock
	cargo build --release

install: out/syntect
	mkdir -p $(XI_PLUGIN_DIR)
	cp -r out/syntect $(XI_PLUGIN_DIR)

clean:
	rm -rf out
	cargo clean

.PHONY: clean install
