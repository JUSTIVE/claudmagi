# Logos

The marks the ⌘K search draws inside each chip (#79): `github.svg` for a pull
request, `linear.svg` for an issue, `claude.svg` for a session.

Taken from [simple-icons](https://github.com/simple-icons/simple-icons) 15.22.0,
whose icon files are CC0-1.0. The marks themselves remain the trademarks of
GitHub, Linear and Anthropic, and are used here only to point at those
services.

gpui renders an SVG as a mask, so only the silhouette matters: the colour comes
from the chip's own label colour. A replacement has to be a single-colour SVG
with a `viewBox`; anything else in the file is ignored.
