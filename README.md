# hyper-revamp

**A Rust re-implementation of [Hyper](https://github.com/vercel/hyper).**

Hyper is a terminal built on Electron and web standards. `hyper-revamp` keeps
the configuration, themes, and behavior, and replaces the runtime: no Electron,
no Chromium, no webview — a native binary rendering on the GPU.

It is a drop-in replacement for the Electron build. It reads and writes the same
config file (`~/.config/hyper-revamp/hyper-revamp.json`, `$XDG_CONFIG_HOME`
honored), the same `themes/` directory, and the same `session-state.json`, so
you can switch between the two without migrating anything.

```
vercel/hyper  →  hyper-revamp (Electron fork)  →  hyper-revamp (this repo, native Rust)
```

## Project goals

Hyper's stated goal is "a beautiful and extensible experience for command-line
interface users, built on open web standards." This project keeps the first half
and trades the second: the web stack is what costs Hyper its startup time,
memory, and input latency, so it's gone.

- **Native and fast** — a single binary, GPU-rendered, no browser engine
- **Config-compatible** — same file formats, same keys, same semantics
- **Faithful** — match the Electron build's behavior everywhere it's worth
  matching, and document every place it deliberately doesn't

The extension API is the explicit casualty. Hyper plugins are npm packages that
inject JavaScript and CSS into a renderer process; there is no renderer here, so
there are no plugins.

### Stack

- **[egui](https://github.com/emilk/egui)/eframe (wgpu)** — GPU-rendered UI
- **[alacritty_terminal](https://crates.io/crates/alacritty_terminal)** — VT
  emulation, PTY handling, and the reader/writer event loop
- Bundled Cascadia Code (regular/bold/italic/bold-italic)

## Usage

There are no package-manager releases yet — no Homebrew cask, no Chocolatey
package, no AUR entry. Build from source with a stable Rust toolchain
([rustup](https://rustup.rs)).

### macOS

```sh
./bundle/bundle-mac.sh            # release .app at dist/hyper-revamp.app
```

This builds `--release`, assembles the bundle, generates the icon, writes
`Info.plist`, and ad-hoc codesigns. Set `TARGET_TRIPLE` to cross-compile
(defaults to `aarch64-apple-darwin`).

The result is ad-hoc signed, which is fine on the machine that built it.
Gatekeeper will block it elsewhere — distributing it needs a Developer ID
signature and notarization.

Optionally install the CLI:

```sh
ln -sf "$PWD/dist/hyper-revamp.app/Contents/MacOS/hyper-revamp" /usr/local/bin/hyper-revamp
```

### Linux

```sh
cargo build --release -p hyper-app     # binary at target/release/hyper-revamp
```

Wayland and X11 are both enabled. There is no packaging script — install the
binary wherever you like.

### Windows

The config layer resolves Windows paths, but there is no bundling script and the
platform is not regularly exercised. `cargo build --release -p hyper-app` is the
whole story.

### Command line

```sh
hyper-revamp [paths…]
```

Opens a tab per path — a directory opens a tab in it, an executable file runs,
any other file is typed at the prompt (the `openFile` rules) — and a path that
doesn't exist is an error. A second invocation hands off to the running instance
over a unix socket. `--version` prints the version.

## Feature parity

Carried over from the Electron build:

- Profiles (local + SSH with publickey/password auth), per-profile themes and
  tab accent colors, profile-aware `preserveCWD`
- 12 bundled themes + user themes (`themes/*.json`, `*.css` with `--hyper-*`
  custom properties), live hot reload on config/theme file changes
- Tabs and splits with the same split/rebalance semantics as `term-groups.ts`,
  drag-resizable separators, ⌘1–9 tab jumps
- Full keymap support (`keymaps` section, mousetrap strings), native macOS menus
  whose accelerators follow the keymap
- Selection-snapshot clipboard behavior, copyOnSelect, quickEdit, bracketed
  paste, mouse reporting (SGR + legacy), scrollback, search (⌘F), URL detection
  (regex + OSC 8) with scheme allowlist
- Settings window (⌘,): visual editors for profiles/themes/terminal/env/keymaps
  plus a raw JSON editor. Edits apply to your own document, never the
  defaults-merged view, so unknown keys (`plugins`, `$schema`, `css`, …) survive
  and defaults are never baked in; each key your file sets has a reset control
  that removes it again
- Command palette: type `/` at the start of a prompt line (`/profile <name>`)
- Session restore (`restoreSession: true`) and window geometry persistence, in
  the same file formats as the Electron build
- ssh:// URL handler (opens a tab and types the ssh command), shell fallback
  warning when a custom shell dies on startup, bell (sound + visual flash),
  focus/blur window opacity

### Deliberate deferrals

Ligatures, sixel/iTerm2 images, screen-reader mode, auto-update, news
notifications, the win/linux hamburger menu, and CSS injection (`css` /
`termCSS` are preserved in the file but have no effect natively). The plugin
system is dropped entirely.

### Deliberate behavior differences

Everywhere else the goal is to match the Electron build exactly. These are the
places it knowingly doesn't:

- **`command`/`cmd`/`meta` bindings don't fire off macOS.** egui has no Super
  modifier, so there is nothing to bind them to. Mapping them onto Ctrl instead
  would swallow Ctrl+C, so they simply don't fire — much as they wouldn't
  without a Super press upstream. The default linux/win32 keymaps use `ctrl+`,
  so nothing shipped is affected.
- **`editor:deleteEndLine` sends `\x0b` (VT), not `\x10B`.** Upstream's value is
  a typo — `\x10` is DLE followed by a literal `B` — that no shell acts on.
  `\x0b` is the kill-to-end-of-line every readline and zle binding expects.
- **Pasted control characters are stripped**, except tab and newline (`\r\n` and
  `\n` normalize to `\r`, so multi-line pastes still run as they do in any
  terminal). Upstream forwards the rest as-is, which lets a paste smuggle escape
  sequences into the shell.
- **Mouse mode 1005 (UTF-8 coordinates) is honored.** xterm.js dropped it, so
  the Electron build never spoke it — but `alacritty_terminal` sets the mode
  whenever an app asks, and replying to a UTF-8 request with X10 bytes is worse
  than answering it properly.
- **The window position is validated a frame late.** Electron checked the saved
  position against the display list before opening the window; winit only
  exposes displays once the event loop is running, so a window restored onto an
  unplugged monitor is placed and then recentered rather than never placed
  wrong.

## Contribute

Regardless of platform, you will need a stable Rust toolchain via
[rustup](https://rustup.rs). Then:

```sh
cargo run -p hyper-app            # debug build, runs the app
cargo test                        # workspace tests
cargo clippy --all-targets        # lints
cargo fmt                         # formatting
```

### Workspace layout

```
crates/
├── hyper-term      # engine: sessions, PTY, ssh, cwd, env (no GUI deps)
├── hyper-config    # config model, themes, keymaps, hot reload
└── hyper-app       # the eframe app (binary name: hyper-revamp)
assets/             # themes, keymaps, schema, fonts, icon — all include_str!'d
bundle/             # bundle-mac.sh
```

Everything under `assets/` is compiled into the binary with `include_str!` /
`include_bytes!`, so adding a bundled theme or keymap means editing both the
file and the table that embeds it (`theme.rs`, `keymap.rs`).

#### Known issues that can happen during development

##### `warning: couldn't generate hyper-revamp.icns`

`bundle-mac.sh` shells out to ImageMagick to build the iconset. Without it the
bundle still builds, just with no icon. Install it with `brew install
imagemagick`.

##### The built `.app` won't open on another Mac

The bundle is ad-hoc signed (`codesign --sign -`). Gatekeeper only trusts that
on the machine that produced it. Real distribution needs a Developer ID
certificate and notarization.

##### First release build is slow

`wgpu` and `eframe` are a large dependency tree; a cold `--release` build takes a
few minutes. Incremental rebuilds are fast. `[profile.dev]` uses `opt-level = 1`
(and `2` for dependencies) so debug builds are usable at speed.

## Related Repositories

- [vercel/hyper](https://github.com/vercel/hyper) — the original, which this
  re-implements
- [alacritty/alacritty](https://github.com/alacritty/alacritty) — source of
  `alacritty_terminal`, the VT emulation core
- [emilk/egui](https://github.com/emilk/egui) — the immediate-mode GUI library
- [Awesome Hyper](https://github.com/bnb/awesome-hyper) — themes and plugins for
  upstream Hyper. Themes are portable here; plugins are not.

## License

MIT, as declared in the workspace `Cargo.toml` and inherited from upstream
Hyper.
