# cliprust

A forever clipboard history for macOS: a native AppKit menu bar app in Rust,
replacing Collective.app (which kept everything in memory, used GBs and lost
history on crashes).

- **⌘⌥V** opens the history (at its last position; first time under the menu bar crab claw).
- Every copy is written to SQLite immediately: all pasteboard representations
  byte-for-byte (zstd-compressed when it helps), source app, window title,
  document path, source URL, timestamps. Recall puts back exactly what was copied.
- Text found in images (Apple Vision OCR) is searchable.
- Runs as a login agent that launchd restarts if it ever crashes.

## Keys

| Key | Action |
| --- | --- |
| ↑ ↓ / PgUp PgDn | move (also while typing in search) |
| ⏎ / double-click | copy to the clipboard and close · ⇧⏎ as plain text (Preferences: "⏎ pastes" pastes into the app in front instead) |
| ⌘F, `/`, or just start typing | search |
| `s` | toggle the starred-only filter |
| space, ⌘S, or click ☆ | star / unstar |
| ⌘1…⌘9 | the nth clip, like ⏎ |
| ⌘⏎ | the other one: paste when ⏎ copies, copy when ⏎ pastes |
| ⌘C | copy to the clipboard |
| ⌫ | delete clip |
| → ← | open / fold the preview pane (or click its button, top right) |
| ⌘T | edit the clip's tags (⏎ or Esc: back to the list) |
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

Search: words match anywhere (text, OCR text, app, window title, path, URL;
case and accent insensitive). Operators: `"exact phrase"`, `-exclude`,
`app:chrome`, `type:image|text|url|file`, `is:star`, `tag:name`.

Preferences also set the text size and an interface size slider (70–125%, in 5% steps below 100%)
that zooms the whole window: text, rows, icons and the window itself.

## Build

`make` lists the targets (`build`, `run`, `test`, `install`, `import-collective`,
`unload-collective`, `release`, `logs`, `stats`).

Data: `~/Library/Application Support/cliprust/cliprust.db` · Log: `~/Library/Logs/cliprust.log`
