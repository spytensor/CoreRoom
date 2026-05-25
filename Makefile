.PHONY: readme-images

# Deprecated since v0.9.13. The README hero image is now a real terminal
# capture (`docs/images/live-room.png`). The Pillow renderer is kept on
# disk for archival reference and one-off design mockups only — do not
# run it for the README. See CLAUDE.md > "README Images" for the
# current policy.
readme-images:
	@echo "[deprecated] make readme-images is no longer used for the README."
	@echo "             docs/images/live-room.png is a real terminal capture."
	@echo "             See CLAUDE.md > README Images for the current policy."
	@echo "             To force the legacy renderer anyway, run:"
	@echo "               python3 scripts/render-readme-images.py"
	@exit 1
