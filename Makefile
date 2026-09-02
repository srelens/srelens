.PHONY: srelens-tui help

DEST_DIR ?= $(HOME)/.local/bin
TARGET_BIN ?= $(DEST_DIR)/srelens-tui

srelens-tui:
	@mkdir -p $(DEST_DIR)
	cargo build --release -p srelens-tui
	cp target/release/srelens-tui $(TARGET_BIN)
	@echo "✓ Successfully built and installed srelens-tui to $(TARGET_BIN)"

help:
	@echo "Available targets:"
	@echo "  make srelens-tui   Build release binary and install to $(DEST_DIR)/srelens-tui"
