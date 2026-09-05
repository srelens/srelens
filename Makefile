.PHONY: srelens-tui help

DEST_DIR ?= $(HOME)/.local/bin
TARGET_BIN ?= $(DEST_DIR)/srelens-tui

srelens-tui:
	@mkdir -p $(DEST_DIR)
	cargo build --release -p srelens-tui
	@rm -f $(TARGET_BIN)
	cp target/release/srelens-tui $(TARGET_BIN)
	@codesign -f -s - $(TARGET_BIN) 2>/dev/null || true
	@echo "✓ Successfully built, signed, and installed srelens-tui to $(TARGET_BIN)"

help:
	@echo "Available targets:"
	@echo "  make srelens-tui   Build release binary and install to $(DEST_DIR)/srelens-tui"
