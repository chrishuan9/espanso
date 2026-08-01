# Using this devcontainer with Podman

`devcontainer.json` in this folder targets the generic devcontainer spec and works
with Docker out of the box. It doesn't need any changes to work with rootless
Podman too, but Podman needs a bit of one-time machine setup that Docker doesn't.

This is **not** baked into `devcontainer.json` itself: some of the flags Podman
needs (e.g. `--userns=keep-id`) aren't understood by Docker, so putting them in the
shared config would break teammates who use Docker. Everything below is local,
per-user configuration instead.

If you hit a hang during **container creation / installing features** specifically
(as opposed to it starting fine but VS Code getting stuck later "waiting for
forwarded ports"), work through this checklist:

## 1. Enable the Podman API socket

VS Code's Dev Containers extension talks to a Docker-API-compatible socket. Podman
ships one, but it isn't running by default:

```sh
systemctl --user enable --now podman.socket
```

## 2. Point VS Code at Podman

Try the `dockerPath` setting first:

```jsonc
// settings.json
{
  "dev.containers.dockerPath": "podman"
}
```

If your version of the Dev Containers extension doesn't pick that up, fall back to
exporting `DOCKER_HOST` before launching VS Code (or in your shell profile):

```sh
export DOCKER_HOST=unix:///run/user/$(id -u)/podman/podman.sock
```

## 3. Rootless UID mapping

Add to `~/.config/containers/containers.conf`:

```ini
[containers]
userns = "keep-id"
```

This is more commonly cited for permission errors than for the container-creation
hang itself, but it's a standard part of a working rootless Podman + VS Code setup,
so worth having in place regardless.

## 4. If it's still hanging: check for a silent registry prompt

This devcontainer uses three **features** (`./features/rust-dependencies`,
`./features/x11-dependencies`, and the remote `desktop-lite` feature), which means
the tooling has to build a derived image applying them as extra layers on top of
the base image. That "extend the base image" build step is where rootless-Podman
setups most commonly hang.

If `/etc/containers/registries.conf` lists more than one entry under
`unqualified-search-registries`, Podman may want to prompt interactively for which
registry to pull from during that build — a prompt VS Code has no way to answer, so
it just hangs with no visible error. Fix by pinning a single registry there, e.g.:

```toml
unqualified-search-registries = ["docker.io"]
```

## Still stuck?

Grab the actual container-creation log via VS Code's command palette:
**"Dev Containers: Show Container Log"**. That will show exactly which step it's
stuck on (image build, feature install, or something else), which narrows things
down a lot faster than guessing from the symptom alone.
