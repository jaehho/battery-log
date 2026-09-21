# battery-log — sampler daemon + desktop dashboard. See README.md.
# The frontend is static and embedded at compile time, so `cargo run` is all
# you need; makepkg only comes in for the AUR package.

.PHONY: help run daemon test fmt build install uninstall package srcinfo dump status clean

PREFIX  ?= $(HOME)/.local
UNITDIR ?= $(or $(XDG_CONFIG_HOME),$(HOME)/.config)/systemd/user

help:            ## this list
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | sed 's/:.*## /\t/' | expand -t18

run:             ## launch the window (debug build)
	cargo run -p battery-log-tauri

daemon:          ## run the sampler in the foreground, fast, for a look at it
	cargo run --bin battery-logd -- --battery-interval 5 --ac-interval 5

test:            ## unit tests (no GUI, no hardware)
	cargo test -p battery-log-core

fmt:
	cargo fmt

build:           ## optimized binaries -> target/release/
	cargo build --release --workspace

install: build   ## install into ~/.local for this user (no sudo, no AUR needed)
	install -Dm755 target/release/battery-log  $(PREFIX)/bin/battery-log
	install -Dm755 target/release/battery-logd $(PREFIX)/bin/battery-logd
	install -Dm644 packaging/battery-log.desktop \
		$(PREFIX)/share/applications/battery-log.desktop
	install -Dm644 src-tauri/icons/128x128.png \
		$(PREFIX)/share/icons/hicolor/128x128/apps/battery-log.png
	install -Dm644 src-tauri/icons/32x32.png \
		$(PREFIX)/share/icons/hicolor/32x32/apps/battery-log.png
	sed 's#^ExecStart=/usr/bin/#ExecStart=$(PREFIX)/bin/#' packaging/battery-logd.service \
		> $(UNITDIR)/battery-logd.service
	systemctl --user daemon-reload
	@echo "installed. start the sampler with: systemctl --user enable --now battery-logd"

uninstall:       ## remove the user install (the log in ~/.local/state stays)
	systemctl --user disable --now battery-logd 2>/dev/null || true
	rm -f $(PREFIX)/bin/battery-log $(PREFIX)/bin/battery-logd \
	      $(PREFIX)/share/applications/battery-log.desktop \
	      $(PREFIX)/share/icons/hicolor/*/apps/battery-log.png \
	      $(UNITDIR)/battery-logd.service
	systemctl --user daemon-reload

package:         ## build the Arch package
	cd packaging && makepkg -f

srcinfo:         ## regenerate packaging/.SRCINFO after a recipe change
	cd packaging && makepkg --printsrcinfo > .SRCINFO

dump:            ## the dashboard as JSON (also what ui/demo.json is)
	cargo run -q -p battery-log-tauri -- dump

status:          ## the text summary
	cargo run -q -p battery-log-tauri -- status

clean:
	cargo clean
	rm -f ui/demo.json
