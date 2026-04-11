#!/usr/bin/env bash

set -euo pipefail

SCRIPT_PATH="${BASH_SOURCE[0]}"
while [ -h "$SCRIPT_PATH" ]; do
  SCRIPT_DIR="$(cd -P "$(dirname "$SCRIPT_PATH")" >/dev/null 2>&1 && pwd)"
  SCRIPT_PATH="$(readlink "$SCRIPT_PATH")"
  [[ "$SCRIPT_PATH" != /* ]] && SCRIPT_PATH="$SCRIPT_DIR/$SCRIPT_PATH"
done

ROOT_DIR="$(cd -P "$(dirname "$SCRIPT_PATH")" >/dev/null 2>&1 && pwd)"
DIST_DIR="$ROOT_DIR/dist"
ARCH_DIR="$DIST_DIR/mac-arm64"
APP_NAME="hyper-revamp"
APP_ID="co.zeit.hyper-revamp"
APP_TEMPLATE="$ROOT_DIR/node_modules/electron/dist/Electron.app"
ARCH_APP="$ARCH_DIR/$APP_NAME.app"
OUTPUT_APP="$DIST_DIR/$APP_NAME.app"
ICON_SOURCE="$ROOT_DIR/app/static/icon.png"
ICNS_PATH="$ARCH_DIR/$APP_NAME.icns"
ICON_BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/$APP_NAME.icns-build.XXXXXX")"

cleanup() {
  rm -rf "$ICON_BUILD_DIR"
}
trap cleanup EXIT

cd "$ROOT_DIR"

if [[ ! -d node_modules ]]; then
  echo "node_modules is missing. Run 'yarn install' first."
  exit 1
fi

if [[ ! -d target/node_modules ]]; then
  echo "target/node_modules is missing. Run 'yarn install' first so postinstall can prepare the app bundle."
  exit 1
fi

if [[ ! -d "$APP_TEMPLATE" ]]; then
  echo "Electron.app template is missing at $APP_TEMPLATE"
  exit 1
fi

if [[ ! -f "$ICON_SOURCE" ]]; then
  echo "App icon source is missing at $ICON_SOURCE"
  exit 1
fi

if [[ -f "$HOME/.nvm/nvm.sh" ]]; then
  # shellcheck disable=SC1090
  source "$HOME/.nvm/nvm.sh"
  nvm use 20 >/dev/null
fi

APP_VERSION="$(node -p "require('./package.json').version")"

make_icns() {
  local size

  rm -f "$ICNS_PATH"

  for size in 16 32 64 128 256 512 1024; do
    magick "$ICON_SOURCE" -background none -resize "${size}x${size}!" "$ICON_BUILD_DIR/${size}x${size}.png"
  done

  node "$ROOT_DIR/bin/make-icns.js" "$ICON_BUILD_DIR" "$ICNS_PATH"
}

stage_bundle() {
  local app_bundle="$1"
  local plist_path="$app_bundle/Contents/Info.plist"
  local resources_dir="$app_bundle/Contents/Resources"
  local macos_dir="$app_bundle/Contents/MacOS"
  local bin_dir="$resources_dir/bin"

  ditto "$APP_TEMPLATE" "$app_bundle"
  mv "$macos_dir/Electron" "$macos_dir/$APP_NAME"

  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName $APP_NAME" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :CFBundleName $APP_NAME" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :CFBundleExecutable $APP_NAME" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $APP_ID" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $APP_VERSION" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $APP_VERSION" "$plist_path"
  /usr/libexec/PlistBuddy -c "Set :LSApplicationCategoryType public.app-category.developer-tools" "$plist_path"

  rm -rf "$resources_dir/app" "$bin_dir"
  mkdir -p "$bin_dir"

  ditto "$ROOT_DIR/target" "$resources_dir/app"
  cp "$ROOT_DIR/bin/cli.js" "$bin_dir/cli.js"
  cp "$ROOT_DIR/bin/yarn-standalone.js" "$bin_dir/yarn-standalone.js"
  cp "$ROOT_DIR/build/mac/hyper" "$bin_dir/$APP_NAME"
  cp "$ROOT_DIR/build/mac/hyper" "$bin_dir/hyper"
  chmod +x "$bin_dir/$APP_NAME" "$bin_dir/hyper"

  if [[ -f "$ICNS_PATH" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleIconFile $APP_NAME.icns" "$plist_path"
    cp "$ICNS_PATH" "$resources_dir/$APP_NAME.icns"
  fi

  codesign --force --deep --sign - --timestamp=none "$app_bundle"
}

echo "==> Building production assets"
rm -rf "$ROOT_DIR/dist/tmp"
rm -f "$ROOT_DIR/target/tsconfig.tsbuildinfo"
yarn run build

echo "==> Preparing macOS arm64 app bundle"
mkdir -p "$ARCH_DIR"
rm -rf "$ARCH_APP" "$OUTPUT_APP"

if ! make_icns; then
  echo "warning: failed to generate $APP_NAME.icns from $ICON_SOURCE; using Electron's default app icon"
fi
stage_bundle "$ARCH_APP"
ditto "$ARCH_APP" "$OUTPUT_APP"

echo
echo "Build complete."
echo "Double-click this app bundle to launch $APP_NAME:"
echo "  $OUTPUT_APP"
