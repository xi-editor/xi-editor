PLUGIN_DIR = ~/.config/xi/plugins

target/release/xi-syntect-plugin:
	cargo build --release

install: manifest.toml target/release/xi-syntect-plugin
	install -D manifest.toml $(PLUGIN_DIR)/syntect/manifest.toml
	install -D target/release/xi-syntect-plugin $(PLUGIN_DIR)/syntect/bin/xi-syntect-plugin

.PHONY: install
