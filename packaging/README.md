# Packaging

AUR package `battery-log-git` — builds both binaries from `main` and installs
the sampler's user service, a launcher entry, and icons.

Files:

- `PKGBUILD` — the AUR recipe (VCS `-git` package, no release tags needed).
- `.SRCINFO` — generated metadata AUR requires (`make srcinfo`).
- `battery-logd.service` — the sampler, a systemd **user** service. Installed
  disabled; `systemctl --user enable --now battery-logd` starts it (the
  dotfiles `stow` phase does this).
- `battery-log.desktop` — launcher entry.

The service is packaged rather than stowed because the binary it runs is
packaged: a unit pointing at `/usr/bin/battery-logd` belongs with the thing it
starts, and `ProtectHome=read-only` cannot follow a symlink into `~/dotfiles`
anyway.
