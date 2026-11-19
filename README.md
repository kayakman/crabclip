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
| ⏎ | paste into the frontmost app · ⇧⏎ paste as plain text |
| ⌘F, `/`, or just start typing | search |
| `s` | toggle the starred-only filter |
| space, ⌘S, or click ☆ | star / unstar |
| ⌘1…⌘9 | paste the nth clip |
| ⌘C / ⌘⏎ | copy to the clipboard without pasting |
| ⌫ | delete clip |
| → ← | open / fold the preview pane (or click its button, top right) |
| ⌘P | pin (window stays open) |
| Esc | clear search, then close |
| ⌘+ / ⌘− / ⌘0 | interface size bigger / smaller / 100% |
| ⌘, | preferences (again: close them) |

Search: words match anywhere (text, OCR text, app, window title, path, URL;
case and accent insensitive). Operators: `"exact phrase"`, `-exclude`,
`app:chrome`, `type:image|text|url|file`, `is:star`.

Preferences also set the text size and an interface size slider (70–125%, in 5% steps below 100%)
that zooms the whole window: text, rows, icons and the window itself.

## Build

`make` lists the targets (`build`, `run`, `test`, `install`, `import-collective`,
`unload-collective`, `release`, `logs`, `stats`).

Data: `~/Library/Application Support/cliprust/cliprust.db` · Log: `~/Library/Logs/cliprust.log`
