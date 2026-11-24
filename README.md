<div align="center">

# Ferric

**A fast, native IRC client with a classic look, built-in FiSH encryption and scripting.**

Rust core · Tauri 2 shell · Windows and Linux

![Rust](https://img.shields.io/badge/Rust-2021%20edition-orange?logo=rust)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-blue)
![License](https://img.shields.io/badge/license-MIT-green)
![Status](https://img.shields.io/badge/status-early%20development-yellow)

[Features](#features) · [Quick start](#quick-start) · [Configuration](#configuration) · [Security](#security) · [How it fits together](#how-it-fits-together) · [License](#license)

<br>

<img src="assets/screenshots/main-dark.png" alt="Ferric main window: server tree, chat with colors, nick list with channel modes, RSS ticker" width="100%">

<sub>Screenshots use demo data only: fake nicks and the reserved domain <code>example.invalid</code>.</sub>

</div>

---

## Why Ferric

- **Your setup, no import.** Server entries come from `connectd.mrc`, FiSH keys from `blow.ini`. Ferric reads them as they are.
- **Encrypted by key.** Messages to a target with a stored FiSH key go out encrypted (ECB or CBC); incoming ones are decrypted. DH1080 key exchange for queries.
- **Classic habits.** Right-click menus, nick colors by channel mode, a tree of options, and the shortcuts you know: **Alt+O** for Options, **Alt+R** for the Scripts Editor.
- **Scripts.** Load `.mrc` script files (a subset of the classic script language) or `.py` Python scripts.
- **Native core.** Connections, IRC parsing, TLS, FiSH, and config handling are plain Rust in the `ferric-core` crate. The interface is a light Vite app, no heavy framework.

<table>
  <tr>
    <td width="50%"><img src="assets/screenshots/nick-menu.png" alt="Right-click menu on a nick: Infos, Status, Kicks and bans, Ignore, Invite, CTCP, FiSH"></td>
    <td width="50%"><img src="assets/screenshots/options.png" alt="Options dialog with a category tree: Connect, IRC, Sounds, Mouse, DCC, Display"></td>
  </tr>
  <tr>
    <td align="center"><sub>Context menus for nicks, channels, queries and the status window</sub></td>
    <td align="center"><sub>Options with a category tree (Alt+O)</sub></td>
  </tr>
</table>

## Features

| Area | What you get |
|---|---|
| **IRC** | Multiple networks, channels, private conversations, nick lists, unread counts, and highlights. IRC color and formatting codes (bold, underline, colors) are rendered. |
| **TLS** | Options → Connect: **Allow self-signed certificates** (on by default) and **Reject invalid certificates** (off by default; it also refuses self-signed ones). A self-signed certificate is trusted on the first connect and its fingerprint is stored. Later connects are refused if the certificate changed. See [Security](#security). |
| **FiSH** | ECB by default. A key stored with the `cbc:` prefix sends in CBC. Incoming CBC messages are recognized by their `+OK *` or `mcps *` prefix, independent of the key. Key exchange via DH1080 for private queries. Manage keys in **Tools → FiSH key manager…** or with `/fish`, `/keys`, and `/delkey`. |
| **Context menus** | Right-click menus for the sidebar, the status window, channels, queries, the nick list, chat senders, and the menu bar. |
| **Nick colors** | Nick list and chat names follow the channel mode (`~`, `&`, `@`, `%`, `+`), first match wins. |
| **Queries** | Double-click a nick or use `/query <nick> [message]` to open a private conversation. `/close` or the close button ends it. |
| **Options** | **Alt+O** opens the Options dialog with a category tree, among them Display, Mouse, Logging, Identd, and Proxy. The Proxy page supports SOCKS4, SOCKS5, and HTTP CONNECT. |
| **Scripts Editor** | **Alt+R** opens an editor for `.mrc` and `.py` files in the `Scripts` folder, with line numbers, backup on save, and external conflict detection. Scripts you load are loaded again at the next start (listed in `loaded-scripts.txt` next to `connectd.mrc`). The `.mrc` interpreter runs a subset of aliases and events; not every command is supported. |
| **Python scripts** | `.py` scripts run in a Python runtime. Ferric uses a `python` folder next to `ferric-app.exe` if it exists, otherwise the Python found on your `PATH`. |
| **Server manager** | **Tools → Server manager…** lists the `server` lines of `connectd.mrc`. Add, edit, enable, disable, or delete servers; comments and other lines stay as they are. A save is refused when the file changed on disk. |
| **RSS ticker** | Optional headline ticker in the toolbar: a fixed set of headlines (default, three at a time) or a scrolling ticker. Feeds go through your proxy settings. |
| **Interface** | Light and dark themes, per-area fonts (**View → Font…**), an optional nick list, a Switchbar toggle, and a notes window. |

<details>
<summary><b>Commands</b></summary>

| Command | Effect |
|---|---|
| `/join #chan` (`/j`) | Join a channel. A missing `#` is added. |
| `/part [#chan]` (`/p`) | Leave the given or the active channel. |
| `/query nick [message]` (`/q`) | Open a private conversation, optionally with a first message. |
| `/close [target]` | Close the given or the active window. |
| `/clear` | Clear the active window. |
| `/raw line` | Send a raw IRC line. |
| `/scriptload file` | Load a `.mrc` or `.py` file from the `Scripts` folder for this session. Invoke a loaded `.mrc` alias with `/name arguments`. Requires an active network and buffer. |
| `/scriptunload file` | Unload a script loaded from that file in this session. |
| `/connectd` (`/load`) | Reload server entries from `connectd.mrc`. |
| `/fish [target] key` (`/key`, `/keys`, `/blowfish`) | Store a FiSH key in `blow.ini`. Without arguments, opens the key manager. |
| `/delkey [target]` | Delete a FiSH key from `blow.ini`. |

Any other line that starts with `/` is sent to the server as a raw IRC command.

</details>

---

## Quick start

### Windows

Prerequisites (check them with `.\build-exe.ps1 -Check`):
- **Node.js & npm**: `winget install OpenJS.NodeJS.LTS`
- **Rust & Cargo**: `winget install Rustlang.Rustup` (default toolchain: `stable-x86_64-pc-windows-msvc`)
- **Microsoft C++ Build Tools**: `winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`
- **WebView2 Runtime**: `winget install Microsoft.EdgeWebView2Runtime` (usually pre-installed on Windows 11)

```powershell
.\build-exe.ps1                      # build the UI, then the release executable
.\target\release\ferric-app.exe      # start Ferric
```

More options:
```powershell
.\build-exe.ps1 -Check               # check Windows prerequisites only (no build)
.\build-exe.ps1 -Debug               # debug build instead of release
.\build-exe.ps1 -Test                # run 'cargo test --workspace' before building
.\build-exe.ps1 -Clean               # clean previous build artifacts
.\build-exe.ps1 -Reinstall           # force clean install of npm dependencies
.\build-exe.ps1 -Run                 # start Ferric directly after a successful build
```

### Linux

Prerequisites: **Rust and Cargo**, **Node.js and npm**, and the Tauri development libraries:
- **Debian/Ubuntu**: `sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config build-essential`
- **Fedora**: `sudo dnf install -y webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel openssl-devel`
- **Arch**: `sudo pacman -S --needed webkit2gtk-4.1 libayatana-appindicator librsvg openssl`

```bash
chmod +x build-linux.sh
./build-linux.sh                     # build the UI, then the release binary
./target/release/ferric-app          # start Ferric
```

More options:
```bash
./build-linux.sh --debug             # debug build instead of release
./build-linux.sh --test              # run 'cargo test --workspace' before building
./build-linux.sh --clean             # clean previous build artifacts
./build-linux.sh --reinstall         # force clean install of npm dependencies
./build-linux.sh --run               # start Ferric directly after a successful build
```

<details>
<summary><b>Manual build and core tests</b></summary>

```bash
cd ui
npm ci
npm run build
cd ..
cargo build --release -p ferric-app
cargo test -p ferric-core
```

| Output | Content |
|---|---|
| `ui/dist/` | The built interface that the app loads (`frontendDist` in `tauri.conf.json`). |
| `target/release/ferric-app.exe` | The Windows executable (or `ferric-app` on Linux). |

</details>

<details>
<summary><b>Try the interface without a backend</b></summary>

The interface starts in a demo mode when it runs in a normal browser instead of the app shell. It shows the screenshots' fake conversation and needs no server:

```bash
cd ui
npm ci
npm run dev
```

Then open the address Vite prints. Connecting, scripts and the key manager need the real app.

</details>

---

## Configuration

Ferric uses two files:

| File | Contents |
|---|---|
| `blow.ini` | FiSH keys. |
| `Scripts\connectd.mrc` | Server entries. The Server manager writes this file; you can also edit it by hand. |

Both files are looked up in the same places, in this order, and the first match wins:

1. The folder in the `FERRIC_CONFIG_DIR` environment variable, if set.
2. The Ferric project folder: the folder of `ferric-app.exe`, or two levels up when the executable runs from `target\release` (the repository root).
3. The app configuration folder of your Windows user account.

Ferric never searches the working directory or parent folders. A missing `connectd.mrc` is created in `Scripts\` of the project folder on the first save; a missing `blow.ini` is created there on the first saved FiSH key.

Next to `connectd.mrc` Ferric keeps three small files: `ferric-servers.txt` (server order and network names), `ferric-tls-pins.txt` (pinned certificate fingerprints), and `loaded-scripts.txt` (scripts to load at start).

> [!IMPORTANT]
> These files can hold passwords and keys. Keep them out of the repository and never share them.

### Example files

The `examples` folder has templates for both files. Copy them to the locations above and remove the `.example` ending:

- `examples\connectd.mrc.example` becomes `Scripts\connectd.mrc`
- `examples\blow.ini.example` becomes `blow.ini`

Then edit the copies with your own servers and keys.

---

## Security

- **Certificates.** With **Allow self-signed certificates** on, a self-signed certificate is trusted on the first connect and its fingerprint is stored in `ferric-tls-pins.txt`. If that certificate changes later, the connect is refused until you remove the old pin. With **Reject invalid certificates** on, self-signed certificates are refused.
- **Only keyed targets are encrypted.** A message is encrypted only when a FiSH key is stored for its target; everything else is plain IRC text.
- **A failed encryption sends nothing.** If FiSH encryption fails, Ferric reports `FiSH encryption failed; message was not sent`.
- **Your files stay private.** `blow.ini`, `connectd.mrc`, and `ferric-tls-pins.txt` are local to your machine; see [Configuration](#configuration).

---

## How it fits together

```mermaid
flowchart LR
    subgraph files["Config and script files"]
        blow["blow.ini<br/>FiSH keys"]
        conn["Scripts/connectd.mrc<br/>server entries"]
        scripts["Scripts/*.mrc, *.py<br/>scripts"]
    end
    ui["ui/<br/>JavaScript + Vite"]
    app["ferric-app<br/>Tauri 2 shell"]
    core["ferric-core<br/>IRC, TLS, FiSH,<br/>config file parsing"]
    net(("IRC servers"))

    ui <-->|Tauri commands| app
    app --> core
    app -->|resolve paths| files
    core -->|read and write| blow
    core -->|read and write| conn
    core -->|Scripts Editor| scripts
    core <-->|TCP or TLS| net
```

| Part | Role |
|---|---|
| `crates/ferric-core` | Connections, IRC protocol and formatting, TLS, FiSH, `blow.ini` and `connectd.mrc` parsing, script files. No Tauri dependency. |
| `crates/ferric-app` | The Tauri 2 desktop shell: finds the config files, runs the Python scripts, and exposes the core to the UI. |
| `ui/` | The JavaScript interface, built with Vite. |

### Repository map

- `crates/` — `ferric-app` (Tauri shell) and `ferric-core` (protocol, TLS, parsing, FiSH).
- `ui/` — Vite and JavaScript interface.
- `Scripts/` — example scripts; the server list `connectd.mrc` is created here.
- `examples/` — templates for the local configuration files.
- `assets/screenshots/` — the images in this README (demo data only).
- [`index.md`](index.md) — repository entry point.

---

## Project status

**Early development.** Features and behavior may change.

- The Windows build via `build-exe.ps1` and the Linux build via `build-linux.sh` are available.
- Display, Mouse, Logging, Identd, and Proxy pages in Options are functional.
- The `.mrc` interpreter covers a subset of the classic script language; see the Scripts Editor row in [Features](#features).

## License

Ferric is released under the [MIT License](LICENSE).
