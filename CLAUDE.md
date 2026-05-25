# Claude Notes

## README Images

The README hero image is a **real terminal capture** of the live room,
not a synthetic mockup. As of v0.9.13:

- `docs/images/live-room.png` is the single README hero image.
- Take captures from a real `cr` session on macOS (iTerm2 / Terminal)
  at retina resolution. Crop with the alt-screen content only — no
  surrounding window chrome or shell scrollback.
- Use the v0.9.12+ binary so the visual reflects the current
  identity colors, top status bar, Team rail, footer hints, and
  mouse-captured sandbox.

The Pillow-based renderer (`scripts/render-readme-images.py` +
`make readme-images`) is **deprecated** and not used by CI or the
release flow. It is kept on disk for archival reference and for
producing one-off design mockups when a real capture is impractical.
Do not regenerate `live-room.png` from the script — replace it with a
fresh real capture instead, and add new screenshots only when there is
a real product state worth showing.
