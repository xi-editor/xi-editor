# Makefile for installing the plugin on macOS and Linux

# The 'official' name of this plugin, displayed in menus etc
PLUGIN_NAME=xi-lsp-plugin
# the name of the plugin binary; this is the same as the name in Cargo.toml
PLUGIN_BIN=xi-lsp-plugin

# On MacOS we just always assume that plugins are in the default location
ifeq ($(shell uname -s), Darwin)
   XI_CONFIG_DIR ?= $(HOME)/Library/Application\ Support/XiEditor
endif

XDG_CONFIG_HOME ?= $(HOME)/.config
XI_CONFIG_DIR ?= $(XDG_CONFIG_HOME)/xi
XI_PLUGIN_DIR ?= $(XI_CONFIG_DIR)/plugins

.PHONY: $(PLUGIN_BIN)
$(PLUGIN_BIN):
	cargo build  --bin xi-lsp-plugin --release

install: manifest.toml $(PLUGIN_BIN)
	install -d $(XI_PLUGIN_DIR)/$(PLUGIN_NAME)/bin
	install manifest.toml $(XI_PLUGIN_DIR)/$(PLUGIN_NAME)
	install ../target/release/$(PLUGIN_BIN) $(XI_PLUGIN_DIR)/$(PLUGIN_NAME)/bin

.PHONY: install
