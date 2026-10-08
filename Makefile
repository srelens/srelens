.PHONY: srectl help

DEST_DIR ?= $(HOME)/.local/bin
TARGET_BIN ?= $(DEST_DIR)/srectl

srectl:
	@mkdir -p $(DEST_DIR)
	cargo build --release -p srectl
	@rm -f $(TARGET_BIN)
	cp target/release/srectl $(TARGET_BIN)
	@codesign -f -s - $(TARGET_BIN) 2>/dev/null || true
	@echo "✓ Successfully built, signed, and installed srectl to $(TARGET_BIN)"

help:
	@echo "Available targets:"
	@echo "  make srectl   Build release binary and install to $(DEST_DIR)/srectl"
