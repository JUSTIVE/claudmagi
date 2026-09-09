#!/bin/zsh
# Builds claudmagi in release mode, wraps it in a macOS .app bundle with an
# icon, ad-hoc signs it, and installs it into /Applications (falling back to
# ~/Applications). usage: tools/bundle.sh [--no-install]
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=claudmagi
DISPLAY_NAME=Claudmagi
BUNDLE_ID=io.github.justive.claudmagi
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/bundle/$NAME.app

cargo build --release
BIN=target/release/$NAME

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/$NAME"

# Icon: render the board as SVG, rasterise, and pack an .icns.
ICONSET=target/bundle/$NAME.iconset
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
"$BIN" --icon target/bundle/icon.svg >/dev/null
swift tools/svg2png.swift target/bundle/icon.svg target/bundle/icon-1024.png 1 >/dev/null
# Round the corners the way macOS expects (icons are not masked automatically).
swift tools/round_icon.swift target/bundle/icon-1024.png target/bundle/icon-rounded.png
for size in 16 32 128 256 512; do
  sips -z $size $size target/bundle/icon-rounded.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double target/bundle/icon-rounded.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/$NAME.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>$DISPLAY_NAME</string>
  <key>CFBundleExecutable</key><string>$NAME</string>
  <key>CFBundleIconFile</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$DISPLAY_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || echo "warning: ad-hoc codesign failed"

if [[ "${1:-}" == "--no-install" ]]; then
  echo "built $APP"
  exit 0
fi

DEST=/Applications/$NAME.app
if ! rm -rf "$DEST" 2>/dev/null || ! cp -R "$APP" "$DEST" 2>/dev/null; then
  DEST=$HOME/Applications/$NAME.app
  mkdir -p "$HOME/Applications"
  rm -rf "$DEST"
  cp -R "$APP" "$DEST"
fi
# Nudge LaunchServices so Spotlight/Launchpad pick it up right away.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST" >/dev/null 2>&1 || true
echo "installed $DEST"
