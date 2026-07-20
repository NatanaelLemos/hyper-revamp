#!/usr/bin/env bash
# Build a distributable macOS .app for hyper-revamp (native Rust build).
# Mirrors the old repo's build-mac-arm.sh: release build → .app assembly →
# icns → Info.plist → ad-hoc codesign.

set -euo pipefail

SCRIPT_PATH="${BASH_SOURCE[0]}"
while [ -h "$SCRIPT_PATH" ]; do
  SCRIPT_DIR="$(cd -P "$(dirname "$SCRIPT_PATH")" >/dev/null 2>&1 && pwd)"
  SCRIPT_PATH="$(readlink "$SCRIPT_PATH")"
  [[ "$SCRIPT_PATH" != /* ]] && SCRIPT_PATH="$SCRIPT_DIR/$SCRIPT_PATH"
done

ROOT_DIR="$(cd -P "$(dirname "$SCRIPT_PATH")/.." >/dev/null 2>&1 && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_NAME="hyper-revamp"
APP_ID="co.zeit.hyper-revamp-rust"
TARGET_TRIPLE="${TARGET_TRIPLE:-aarch64-apple-darwin}"
OUTPUT_APP="$DIST_DIR/$APP_NAME.app"
ICON_SOURCE="$ROOT_DIR/assets/icon.png"
ICON_BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/$APP_NAME.icns-build.XXXXXX")"

cleanup() {
  rm -rf "$ICON_BUILD_DIR"
}
trap cleanup EXIT

cd "$ROOT_DIR"

APP_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"

echo "==> Building release binary ($TARGET_TRIPLE)"
cargo build --release --target "$TARGET_TRIPLE" -p hyper-app

BINARY="$ROOT_DIR/target/$TARGET_TRIPLE/release/$APP_NAME"
[[ -f "$BINARY" ]] || { echo "release binary missing at $BINARY"; exit 1; }

make_icns() {
  local iconset="$ICON_BUILD_DIR/$APP_NAME.iconset"
  mkdir -p "$iconset"
  local size
  for size in 16 32 128 256 512; do
    magick "$ICON_SOURCE" -background none -resize "${size}x${size}!" "$iconset/icon_${size}x${size}.png"
    local double=$((size * 2))
    magick "$ICON_SOURCE" -background none -resize "${double}x${double}!" "$iconset/icon_${size}x${size}@2x.png"
  done
  iconutil -c icns "$iconset" -o "$ICON_BUILD_DIR/$APP_NAME.icns"
}

echo "==> Assembling $OUTPUT_APP"
rm -rf "$OUTPUT_APP"
mkdir -p "$OUTPUT_APP/Contents/MacOS" "$OUTPUT_APP/Contents/Resources"

cp "$BINARY" "$OUTPUT_APP/Contents/MacOS/$APP_NAME"

ICON_OK=false
if command -v magick >/dev/null 2>&1 && make_icns; then
  cp "$ICON_BUILD_DIR/$APP_NAME.icns" "$OUTPUT_APP/Contents/Resources/$APP_NAME.icns"
  ICON_OK=true
else
  echo "warning: couldn't generate $APP_NAME.icns from $ICON_SOURCE (is imagemagick installed?)"
fi

cat > "$OUTPUT_APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleExecutable</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$APP_ID</string>
  <key>CFBundleShortVersionString</key><string>$APP_VERSION</string>
  <key>CFBundleVersion</key><string>$APP_VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
$( $ICON_OK && echo "  <key>CFBundleIconFile</key><string>$APP_NAME.icns</string>" )
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>SSH URL</string>
      <key>CFBundleURLSchemes</key><array><string>ssh</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

echo "==> Codesigning (ad-hoc)"
codesign --force --deep --sign - --timestamp=none "$OUTPUT_APP"

echo
echo "Build complete."
echo "Double-click this app bundle to launch $APP_NAME:"
echo "  $OUTPUT_APP"
echo
echo "Optional: install the CLI with"
echo "  ln -sf \"$OUTPUT_APP/Contents/MacOS/$APP_NAME\" /usr/local/bin/$APP_NAME"
