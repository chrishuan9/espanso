# linux

Espanso on Linux comes in two different flavors: one for X11 and one for Wayland.
The variant you build or install **must match your active session**, otherwise Espanso may install but silently fail to work.

#### Determining your session type

Run the following command to check whether you are running X11 or Wayland:

```console
echo $XDG_SESSION_TYPE
```

This prints either `x11` or `wayland`. If the variable is empty, follow [these steps to determine which one you are running](https://unix.stackexchange.com/a/325972).

If you are on Wayland (the default on Fedora and recent GNOME-based distributions), build or install the Wayland variant. If you are on X11, use the X11 variant.

#### Necessary dependencies

If compiling on a version of Ubuntu X11 before 22.04 (including 22.04):

```console
sudo apt install libx11-dev libxtst-dev libxkbcommon-dev libdbus-1-dev libwxgtk3.*-dev
```

If compiling on a version of Ubuntu X11 after 22.04:

```console
sudo apt install libx11-dev libxtst-dev libxkbcommon-dev libdbus-1-dev libwxgtk3.*-dev
```

#### Compiling for X11

##### X11 AppImage

The AppImage is a convenient format to distribute Linux applications, as besides the binary,
it also bundles all the required libraries.

You can create the AppImage by running (this will work on X11 systems):

```console
cargo build --release --no-default-features --features modulo,vendored-tls
./scripts/create_app_image.sh
```

You will find the resulting AppImage in the `target/linux/AppImage/out` folder.

##### Binary

You can build the Espanso binary on X11 by running the following command:

```console
cargo build --release --no-default-features --features modulo,vendored-tls
```

You'll then find the `espanso` binary in the `target/release` directory.

#### Compiling on Wayland

You can build Espanso on Wayland by running:

```bash
cargo build --release --no-default-features --features wayland,modulo,vendored-tls
```

You'll then find the `espanso` binary in the `target/release` directory.

#### Running the Wayland build

The Wayland variant detects keystrokes and injects text through `evdev`/`uinput`
instead of X11 APIs, which means it needs raw access to input devices that a
normal user account doesn't have by default. If Espanso builds and starts (you
see the tray icon, if enabled) but expansions never trigger, this permission
issue is almost always the reason — it fails silently, with no error printed
(though `espanso daemon` in the foreground does print a `CAP_DAC_OVERRIDE`
warning explaining it, see below).

- **Injecting text** (`/dev/uinput`) usually already works out of the box on
  systemd-based distros: `systemd-logind` grants the active seat's user an ACL
  on `/dev/uinput` (via the `uaccess` udev tag) automatically. You can confirm
  this with `getfacl /dev/uinput` — you should see a `user:<you>:rw-` entry. If
  it's missing, you'll need a udev rule granting your user or the `input`
  group access to `/dev/uinput` as well.

- **Reading keystrokes** (`/dev/input/eventN`) needs one of the two options
  below. Espanso runs entirely as your own user either way — neither option
  requires running Espanso itself as root or with `sudo`, and both work fine
  under `systemctl --user`.

  **Option A: grant the binary `CAP_DAC_OVERRIDE` (recommended).** This is
  what Espanso's own code is built to use: at startup, if the binary carries
  the capability, it briefly raises it just to open the `/dev/input/*`
  devices, then drops it immediately afterward for the rest of the process's
  lifetime — no standing elevated access, and no group membership needed:

  ```console
  sudo setcap cap_dac_override+p ./target/release/espanso
  getcap ./target/release/espanso   # confirm: cap_dac_override=p
  ```

  The capability is stored as a file attribute on that exact binary, so it
  survives `sudo`-less runs and works identically whether you launch it
  directly, via `systemctl --user`, or otherwise — but it does **not** survive
  a rebuild, since `cargo build` produces a brand new file. Re-run `setcap`
  after every rebuild (and again after `espanso service register`, or
  whenever you point the systemd unit at a different binary).

  **Option B: add your user to the `input` group.** Simpler to set up once,
  but broader/standing access (any process you run can read raw keyboard
  input from then on, not just Espanso), and it doesn't need reapplying after
  rebuilds:

  ```console
  sudo usermod -aG input $(whoami)
  ```

  Group membership is only picked up on your *next* login, so either log out
  and back in, or run `newgrp input` in the specific terminal you'll launch
  Espanso from to pick it up immediately without a full logout. Verify with
  `groups` that `input` is listed before (re)starting Espanso from that shell.

#### Using nix for compilation

We do have nix for building the repo and flakes to track the dependencies.

Do this to build the `release` mode

```
nix build
```

The binary will be located at `/your-espanso-repo-folder/result/bin/espanso`

And this to run:

```
nix run
```

