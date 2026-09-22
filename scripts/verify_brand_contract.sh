#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

fail() {
    echo "Brand contract failed: $1" >&2
    exit 1
}

require_file() {
    [ -f "$1" ] || fail "missing $1"
}

require_text() {
    local file="$1"
    local pattern="$2"
    rg -q "$pattern" "$file" || fail "$file does not contain $pattern"
}

forbid_text() {
    local pattern="$1"
    shift
    if rg -n "$pattern" "$@" >&2; then
        fail "legacy or redundant brand copy remains"
    fi
}

require_file brand/freshloop-mark.svg
require_file brand/freshloop-app-icon.svg
require_file docs/freshloop-brand-system.md
require_file frontend/public/brand/freshloop-mark.svg
require_file frontend/public/icon-192.png
require_file frontend/public/icon-512.png
require_file frontend/public/icon-maskable-512.png
require_file android_client/assets/brand/freshloop-mark.png
require_file android_client/assets/brand/freshloop-app-icon.png
require_file android_client/assets/brand/freshloop-app-foreground.png
require_file android_client/assets/brand/freshloop-app-monochrome.png

require_text android_client/android/app/src/main/AndroidManifest.xml 'android:label="FreshLoop"'
require_text android_client/ios/Runner/Info.plist '<string>FreshLoop</string>'
require_text android_client/ios/Runner.xcodeproj/project.pbxproj 'ASSETCATALOG_COMPILER_GENERATE_SWIFT_ASSET_SYMBOL_EXTENSIONS = YES;'
require_text android_client/android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml '<monochrome'
require_text android_client/flutter_launcher_icons.yaml 'adaptive_icon_monochrome:'
require_text android_client/flutter_launcher_icons.yaml 'ios: false'
require_text frontend/public/manifest.json '"name": "FreshLoop"'
require_text frontend/public/manifest.json '"purpose": "any maskable"'
require_text frontend/public/manifest.json '"background_color": "#050a07"'
require_text frontend/app/layout.tsx 'description: "让重要信息，进入你的循环"'
require_text android_client/lib/ui/feed_screen.dart 'FreshLoopBrandMark'
require_text frontend/app/feed/page.tsx '>周汇总<'
require_text android_client/lib/ui/reading_screen.dart "'周汇总'"
require_text frontend/components/FreshLoopNav.tsx 'compactLabel: "Reading"'

forbid_text \
    'Fresh Loop|FreshLoop Zen Reading|Zen Reading|Audio [Bb]riefing|RADIO \+ READING|LOOP \+ FOCUS|Android Client|Weekly Brief|FreshLoop Weekly|本周精选汇总' \
    frontend/app/layout.tsx \
    frontend/app/page.tsx \
    frontend/app/feed/page.tsx \
    frontend/app/loop/page.tsx \
    frontend/app/focus/page.tsx \
    frontend/public/manifest.json \
    android_client/lib/main.dart \
    android_client/lib/feed_api.dart \
    android_client/lib/ui/feed_screen.dart \
    android_client/lib/ui/reading_screen.dart \
    android_client/lib/audio_handler.dart \
    android_client/vendor/audio_service/android/src/main/java/com/ryanheise/audioservice/AudioService.java \
    android_client/android/app/src/main/AndroidManifest.xml \
    android_client/ios/Runner/Info.plist

forbid_text \
    'ASSETCATALOG_COMPILER_GENERATE_SWIFT_ASSET_SYMBOL_EXTENSIONS = AppIcon' \
    android_client/ios/Runner.xcodeproj/project.pbxproj

node - <<'NODE'
const fs = require("node:fs");
const files = [
  "frontend/public/logo.png",
  "frontend/public/icon-192.png",
  "frontend/public/icon-512.png",
  "frontend/public/icon-maskable-512.png",
  "android_client/assets/brand/freshloop-mark.png",
  "android_client/assets/brand/freshloop-app-icon.png",
  "android_client/assets/brand/freshloop-app-foreground.png",
  "android_client/assets/brand/freshloop-app-monochrome.png",
];
const pngSignature = "89504e470d0a1a0a";
for (const file of files) {
  const signature = fs.readFileSync(file).subarray(0, 8).toString("hex");
  if (signature !== pngSignature) {
    throw new Error(`${file} is not a real PNG`);
  }
}

const appIconDir =
  "android_client/ios/Runner/Assets.xcassets/AppIcon.appiconset";
const appIconContents = JSON.parse(
  fs.readFileSync(`${appIconDir}/Contents.json`, "utf8"),
);
for (const icon of appIconContents.images) {
  const file = `${appIconDir}/${icon.filename}`;
  const png = fs.readFileSync(file);
  const expectedSize =
    Number.parseFloat(icon.size.split("x")[0]) *
    Number.parseInt(icon.scale, 10);
  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);
  if (width !== expectedSize || height !== expectedSize) {
    throw new Error(
      `${file} has ${width}x${height}, expected ${expectedSize}x${expectedSize}`,
    );
  }
}
NODE

node - <<'NODE'
const fs = require("node:fs");
const pubspec = fs.readFileSync("android_client/pubspec.yaml", "utf8");
const versionMatch = pubspec.match(/^version:\s+([^+\s]+)\+(\d+)\s*$/m);
if (!versionMatch) {
  throw new Error("android_client/pubspec.yaml has no valid version");
}

const versionManifest = JSON.parse(
  fs.readFileSync("frontend/public/version.json", "utf8"),
);
const [, versionName, buildNumber] = versionMatch;
if (
  versionManifest.version !== versionName ||
  versionManifest.build_number !== Number(buildNumber)
) {
  throw new Error(
    `Android update metadata mismatch: pubspec=${versionName}+${buildNumber}, ` +
      `version.json=${versionManifest.version}+${versionManifest.build_number}`,
  );
}
if (!String(versionManifest.download_url).includes(":8443/android-app.apk")) {
  throw new Error("Android update download URL must use the Caddy 8443 entrypoint");
}
NODE

echo "Brand contract verified."
