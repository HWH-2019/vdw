.PHONY: all build install install-quick uninstall clean test help dashboard-build dashboard-verify

dashboard-build:
	bash ../scripts/build_dashboard.sh

dashboard-verify:
	python3 ../scripts/verify_dashboard_assets.py

BINARY_NAME := vdw
INSTALL_DIR ?= ~/.local/bin

all: build

build: dashboard-verify
	cargo build --release

# Full install: runs install.sh (auto-detects platform, checks deps, handles all edge cases)
install:
	./install.sh

# Quick install (for devs who already have Rust): build + copy binary globally
install-quick: build
	@echo "Installing $(BINARY_NAME)..."
	@mkdir -p $(INSTALL_DIR)
	@cp target/release/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)
	@# Clear EDR quarantine (causes SIGKILL on some macOS setups)
	@xattr -d com.alibaba.security.edr.antivirus.cloudquery $(INSTALL_DIR)/$(BINARY_NAME) 2>/dev/null || true
	@echo ""
	@echo "  $$(cd $(INSTALL_DIR) && pwd)/$(BINARY_NAME)"
	@echo ""
	$$(cd $(INSTALL_DIR) && pwd)/$(BINARY_NAME) --version
	@echo ""
	@echo "Try: vdw --help"

uninstall:
	@rm -f $(INSTALL_DIR)/$(BINARY_NAME)
	@echo "$(BINARY_NAME) removed from $(INSTALL_DIR)"

clean:
	cargo clean

test:
	cargo test

test-all:
	cargo test -- --ignored

# Delegate to install script with all options
script-install:
	./install.sh $(ARGS)

help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  build          Build release binary"
	@echo "  install        Full install (recommended) — install.sh with platform detection"
	@echo "  install-quick  Quick install for devs with Rust toolchain"
	@echo "  uninstall      Remove from install directory"
	@echo "  clean          Clean build artifacts"
	@echo "  test           Run all tests"
	@echo "  test-all       Run all tests including integration"
	@echo ""
	@echo "Examples:"
	@echo "  make install                    # Full install"
	@echo "  make install INSTALL_DIR=/opt   # Custom directory"
	@echo "  make install-quick              # Quick (build only, skip checks)"
	@echo "  make ARGS='--dir /opt' script-install  # Via install.sh with args"
