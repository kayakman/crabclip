# crabclip 🦀📋

A forever clipboard history for macOS: a native AppKit menu bar app written in
Rust. Every copy goes straight to a local SQLite database, so nothing is lost
on a crash or restart, and years of history stay searchable without eating
memory.

- **⌘⌥V** opens the history (at its last position; first time under the menu bar crab claw).
- Every copy is written to SQLite immediately: all pasteboard representations
  byte-for-byte (zstd-compressed when it helps), source app, window title,
  document path, source URL, timestamps. Recall puts back exactly what was copied.
- Text found in images (Apple Vision OCR) is searchable.
- Tags, stars, an app filter, and an encrypted `password` tag for secrets.
- Snippet folders: file clips in nested folders and browse them from the
  keyboard (type `/`), or with their own global shortcut.
- Imports the history of Clipy (history and snippet folders) or Collective.app
  if you are coming from one of them.
- Runs as a login agent that launchd restarts if it ever crashes.
- Everything stays on your Mac: no network access, no telemetry.

## Keys

| Key | Action |
| --- | --- |
| ↑ ↓ / PgUp PgDn | move (also while typing in search) |
| ⏎ / double-click | copy to the clipboard and close · ⇧⏎ as plain text (Preferences: "⏎ pastes" pastes into the app in front instead) |
| ⌘F, or just start typing | search |
| `/` | browse the snippet folders (below) |
| ⇧⌘S or click the ☆ next to the app filter | starred-only filter (off again at every launch) |
| space, ⌘S, or click ☆ | star / unstar |
| ⌘1…⌘9 | the nth clip, like ⏎ |
| ⌘⏎ | the other one: paste when ⏎ copies, copy when ⏎ pastes |
| ⌘C | copy to the clipboard |
| ⌫ | delete clip (undo for 4 s: ⌘Z or the pill in the bottom bar) |
| → ← | open / fold the preview pane (or click its button, top right) |
| ⌘T | edit the clip's tags (⏎ or Esc: back to the list) |
| ⌘L | file the clip in folders, e.g. `Clients/ssh` (⏎ or Esc: back to the list) |
| ⌘P | pin (window stays open) |
| Esc | clear search, then close |
| ⌘+ / ⌘− / ⌘0 | interface size bigger / smaller / 100% |
| ⌘, | preferences (again, or ⌘W: close them and go back to the history) |

Tags: shown as pills on the clip and under the preview. `url` is added to every
link automatically. Tagging a clip `password` encrypts it (XChaCha20-Poly1305,
key in the login Keychain): the list shows dots, the preview hides it, its text
leaves the search index, ⏎ still copies it (marked concealed so other clipboard
tools skip it); removing the tag stores it in the clear again. Sealing wipes the old copy from
the database files (secure_delete, index rewrite, WAL checkpoint), but backups
made before (Time Machine, APFS snapshots) still hold it. While typing in
search, matching tags drop down: Tab or a click filters by that tag.

Folders: a clip can be filed in one or more folders, nested as deep as you
like (⌘L, paths like `Clients/Formosa/ssh`, completed as you type). A search
starting with `/` browses them instead of the history: `/` lists the top
level, ↑ ↓ move, → opens the selected folder (the search becomes
`/Clients/`), ← goes back up, and text after the last `/` filters the
folder. While a folder is selected the right pane shows what it holds; on a
snippet it shows the value and ⏎ copies it like any clip. Preferences can set
a second global shortcut that opens straight into the folders. Filed clips
are never pruned, like starred ones; a folder goes away when its last clip
leaves it.

Clipy: Preferences › Import from Clipy brings in its history (every format of
each clip) and its snippets, as folders in Clipy's order with their titles
(asked once at launch when Clipy's data is found; `crabclip --import-clipy`
from a terminal). It reads the current Clipy's `sqlite.db`; for an older
Clipy, open the current version once (it converts its data) or import a
snippets export (Clipy › Edit Snippets… › Export, an `.xml` file).

Search: words match anywhere (text, OCR text, app, window title, path, URL;
case and accent insensitive). Operators: `"exact phrase"`, `-exclude`,
`app:chrome`, `type:image|text|url|file`, `is:star`, `tag:name`. Folder
names and snippet titles are searchable words too.

Preferences also set the text size and an interface size slider (70–125%, in 5% steps below 100%)
that zooms the whole window: text, rows, icons and the window itself.

## Install

Requires macOS 13 or later and a Rust toolchain ([rustup](https://rustup.rs)).

```sh
git clone https://github.com/rodrigolive/crabclip
cd crabclip
make install    # release build, signed, copied to /Applications and started
```

The build is signed with the first "Developer ID Application" identity in your
keychain, or ad-hoc if there is none (`SIGN_ID=- make install` forces ad-hoc).
With an ad-hoc signature macOS may ask again for permissions after a rebuild.

Permissions: **Accessibility** is only needed for "⏎ pastes" (it sends ⌘V to
the app in front); without it crabclip copies and you paste. Importing from
Collective reads its sandbox folder, so macOS asks once for access to other
apps' data.

## Build

`make` lists the targets (`build`, `run`, `test`, `install`, `uninstall`,
`import-collective`, `import-clipy`, `unload-collective`, `release`, `logs`, `stats`).

Data: `~/Library/Application Support/crabclip/crabclip.db` · Log: `~/Library/Logs/crabclip.log`

## License

[MIT](LICENSE)
