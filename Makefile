# cliprust — forever clipboard history for macOS (Rust + AppKit)
APP       := cliprust
BUNDLE_ID := com.rod.cliprust
VERSION   := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUILD_NO  := $(shell date +%Y%m%d%H%M)
OUT       := build/$(APP).app
INSTALL   := /Applications/$(APP).app
SIGN_ID   ?= Developer ID Application: Data Giant S.L. (A7YG922U7G)
COLLECTIVE_DIR := $(HOME)/Library/Containers/com.generation-loss.collective/Data/Library/Application Support/Collective

.DEFAULT_GOAL := help
.PHONY: help build bundle run test install uninstall import-collective unload-collective release logs stats icons clean

help: ## Show this help
	@echo "cliprust $(VERSION) — forever clipboard history (menu bar app, ⌘⌥V)"
	@echo
	@echo "Usage: make <target>"
	@echo
	@awk 'BEGIN{FS=":.*## "} /^[a-zA-Z_-]+:.*## /{printf "  \033[1m%-18s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo
	@echo "Data:  ~/Library/Application Support/cliprust/cliprust.db (SQLite, zstd blobs, FTS5 trigram)"
	@echo "Logs:  ~/Library/Logs/cliprust.log"
	@echo "Sign:  SIGN_ID='$(SIGN_ID)' (override with SIGN_ID=- for ad-hoc)"

build: bundle ## Release build + signed build/cliprust.app

bundle:
	cargo build --release
	rm -rf "$(CURDIR)/$(OUT)"
	mkdir -p $(OUT)/Contents/MacOS $(OUT)/Contents/Resources $(OUT)/Contents/Library/LaunchAgents
	cp target/release/$(APP) $(OUT)/Contents/MacOS/$(APP)
	sed -e 's/__VERSION__/$(VERSION)/' -e 's/__BUILD__/$(BUILD_NO)/' bundle/Info.plist > $(OUT)/Contents/Info.plist
	cp bundle/com.rod.cliprust.agent.plist $(OUT)/Contents/Library/LaunchAgents/
	cp assets/AppIcon.icns assets/StatusIcon.png assets/StatusIcon@2x.png $(OUT)/Contents/Resources/
	codesign --force --options runtime --timestamp=none --sign "$(SIGN_ID)" $(OUT)
	codesign --verify --strict $(OUT) && echo "built $(OUT)"

run: build ## Build and run from build/ (quits a running copy first)
	-@pkill -x $(APP); sleep 1
	open $(OUT)

test: ## Run the test suite
	cargo test

install: build ## Install to /Applications and (re)start it; login agent keeps it running
	-@osascript -e 'tell application id "$(BUNDLE_ID)" to quit' 2>/dev/null; sleep 1; pkill -x $(APP) 2>/dev/null; sleep 1
	rm -rf "$(INSTALL)"
	cp -R $(OUT) "$(INSTALL)"
	@if launchctl print gui/$$(id -u)/com.rod.cliprust.agent >/dev/null 2>&1; then \
		launchctl kickstart -k gui/$$(id -u)/com.rod.cliprust.agent; echo "restarted login agent"; \
	else open "$(INSTALL)"; fi
	@echo "installed $(INSTALL)"

uninstall: ## Quit and remove /Applications/cliprust.app (keeps your history db)
	-@pkill -x $(APP)
	-launchctl bootout gui/$$(id -u)/com.rod.cliprust.agent 2>/dev/null
	rm -rf "$(INSTALL)"

import-collective: ## Import Collective.app's saved history (quit Collective first so it saves)
	target/release/$(APP) --import-collective

unload-collective: ## Quit Collective (lets it save), remove it from login items and its 2-hourly restart cron
	-osascript -e 'tell application "Collective" to quit'
	@for i in $$(seq 1 300); do pgrep -x Collective >/dev/null || break; sleep 1; done; \
	 pgrep -x Collective >/dev/null && echo "Collective still running (saving?)" || echo "Collective quit"
	-osascript -e 'tell application "System Events" to delete login item "Collective"'
	-crontab -l 2>/dev/null | sed 's|^\([^#].*collective-restart.sh.*\)$$|# disabled for cliprust: \1|' | crontab -
	defaults write com.generation-loss.collective launchAtLogin -bool false

release: test build ## Tests, signed build, zip into dist/
	mkdir -p dist
	rm -f "$(CURDIR)/dist/$(APP)-$(VERSION).zip"
	ditto -c -k --keepParent $(OUT) dist/$(APP)-$(VERSION).zip
	@echo "dist/$(APP)-$(VERSION).zip"

logs: ## Follow the log
	tail -f ~/Library/Logs/cliprust.log

stats: ## Database stats
	target/release/$(APP) --stats

icons: ## Rebuild AppIcon.icns from assets/AppIcon.png
	rm -rf "$(CURDIR)/.agent/iconset"; mkdir -p .agent/iconset/AppIcon.iconset
	for s in 16 32 128 256 512; do \
		sips -z $$s $$s assets/AppIcon.png --out .agent/iconset/AppIcon.iconset/icon_$${s}x$${s}.png >/dev/null; \
		sips -z $$((s*2)) $$((s*2)) assets/AppIcon.png --out .agent/iconset/AppIcon.iconset/icon_$${s}x$${s}@2x.png >/dev/null; \
	done
	iconutil -c icns .agent/iconset/AppIcon.iconset -o assets/AppIcon.icns

clean: ## Remove build outputs
	cargo clean
	rm -rf "$(CURDIR)/build" "$(CURDIR)/dist"
