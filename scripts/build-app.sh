#!/bin/zsh
# Builds "CastGlow.app" (Intel + Apple Silicon) into ./dist. Add --install to copy it to /Applications.
set -e
cd "$(dirname "$0")/.."
[ -f ~/.cargo/env ] && source ~/.cargo/env
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

APP="dist/CastGlow.app"
rm -rf dist && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create target/aarch64-apple-darwin/release/quadcast target/x86_64-apple-darwin/release/quadcast \
     -output "$APP/Contents/MacOS/CastGlow"

cp assets/AppIcon-512.png icon_512.png
rm -rf AppIcon.iconset && mkdir AppIcon.iconset
for s in 16 32 128 256 512; do
  sips -z $s $s icon_512.png --out AppIcon.iconset/icon_${s}x${s}.png >/dev/null
  d=$((s*2)); [ $d -le 512 ] && sips -z $d $d icon_512.png --out AppIcon.iconset/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns AppIcon.iconset -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>CastGlow</string>
  <key>CFBundleDisplayName</key><string>CastGlow</string>
  <key>CFBundleIdentifier</key><string>com.ushersoft.castglow</string>
  <key>CFBundleExecutable</key><string>CastGlow</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.1.0</string>
  <key>CFBundleVersion</key><string>5</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>CastGlow listens for the near-silence your QuadCast S sends when you tap its mute button, so it can show the mute in the app. Sound is checked in memory only and is never recorded or sent anywhere.</string>
</dict>
</plist>
EOF
codesign --force --deep -s - "$APP"
codesign --verify --deep "$APP"
lipo -archs "$APP/Contents/MacOS/CastGlow"

# Release download (zip keeps the app intact)
ditto -c -k --keepParent "$APP" "dist/CastGlow-macOS.zip"

# Install on this Mac only when asked:  ./scripts/build-app.sh --install
if [ "$1" = "--install" ]; then
  pkill -x CastGlow 2>/dev/null || true; sleep 1
  rm -rf "/Applications/CastGlow.app"
  ditto "$APP" "/Applications/CastGlow.app"
  echo "Installed to /Applications/CastGlow.app"
fi
rm -rf AppIcon.iconset icon_512.png
echo "Built $APP and dist/CastGlow-macOS.zip"
