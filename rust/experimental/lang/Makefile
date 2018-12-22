# Makefile for installing the plugin on macOS and Linux

# The 'official' name of this plugin, displayed in menus etc
PLUGIN_NAME = xi-lang-plugin
# the name of the plugin binary; this is the same as the name in Cargo.toml
PLUGIN_BIN = xi-lang

# On MacOS we just always assume that plugins are in the default location
ifeq ($(shell uname -s), Darwin)
   XI_CONFIG_DIR ?= $(HOME)/Library/Application\ Support/XiEditor
endif

XDG_CONFIG_HOME ?= $(HOME)/.config
XI_CONFIG_DIR ?= $(XDG_CONFIG_HOME)/xi
XI_PLUGIN_DIR ?= $(XI_CONFIG_DIR)/plugins

out/$(PLUGIN_NAME): $(PLUGIN_BIN)
	mkdir -p out/$(PLUGIN_NAME)/bin
	cp ../../target/release/$(PLUGIN_BIN) out/$(PLUGIN_NAME)/bin
	cp manifest.toml out/$(PLUGIN_NAME)/manifest.toml

.PHONY: $(PLUGIN_BIN)
$(PLUGIN_BIN):
	cargo build --release

install: manifest.toml out/$(PLUGIN_NAME)
	mkdir -p $(XI_PLUGIN_DIR)
	cp -r out/$(PLUGIN_NAME) $(XI_PLUGIN_DIR)

clean:
	rm -rf out
	cargo clean

.PHONY: clean install
