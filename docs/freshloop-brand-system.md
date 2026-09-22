# FreshLoop Brand System

FreshLoop is one product with four modes. The brand should feel calm, useful,
audio-aware, and recognizably green without turning every surface into a
marketing banner.

## Brand Architecture

- Master brand: `FreshLoop`
- Product modes: `Radio`, `Reading`, `Loop`, `Focus`
- Content labels: news, curated reading, daily playlists, and weekly digests

Always spell the product name `FreshLoop`. Do not use `Fresh Loop`. The product
modes are navigation labels, not separate sub-brands.

## Brand Promise

Primary Chinese line:

> 让重要信息，进入你的循环

English equivalent:

> Keep what matters in your loop.

The line belongs in install metadata, onboarding, About, and marketing
surfaces. It does not belong in the persistent app header, player, or media
notification.

## Marks

- `brand/freshloop-mark.svg`: transparent primary mark for headers and small UI
  identity.
- `brand/freshloop-app-icon.svg`: opaque dark app icon source.
- `brand/freshloop-status.svg`: compact monochrome audio status mark.

The app icon has no text and no extra outer ring. Platform masks provide the
final silhouette. The primary mark keeps the loop, leaf motion, and audio pulse
as one family.

Regenerate raster and platform source assets with:

```bash
node scripts/generate_brand_assets.mjs
cd android_client
dart run flutter_launcher_icons
```

The Node step owns Web/PWA assets, the complete iOS AppIcon set, launch images,
and Android source images. The Flutter step only regenerates Android launcher
resources; iOS generation stays disabled there to avoid rewriting unrelated
Xcode asset settings.

Then run:

```bash
./scripts/verify_brand_contract.sh
```

## Color Roles

- Brand primary: `#19E66B`
- Base background: `#050A07`
- Primary surface: `#1A3224`
- Highlight surface: `#244732`
- Primary text: `#FFFFFF`
- Muted brand text: `#93C8A8`

Use the green for selection, playback, and primary actions. Do not use it for
every icon or every line of copy.

## Icon Language

Use rounded Material icons for product and functional UI:

- Radio: `radio`
- Reading: `menu_book`
- Loop: `repeat`
- Focus: `adjust`

Outlined icons are the default. Filled icons are reserved for selected states,
active playback, and primary actions. A single action must keep the same symbol
across Web and Android.

## Header And Copy

The persistent header contains only the primary mark, the `FreshLoop` wordmark,
account/download actions, and navigation. Do not repeat the selected product
mode below the wordmark.

Keep `Radio`, `Reading`, `Loop`, and `Focus` as product names. Use Chinese for
operational copy, state messages, and controls in the Chinese interface.

## Release Checklist

- Verify the mark at 16, 24, 40, 192, 512, and 1024 pixels.
- Verify Android adaptive masks and Android 13 themed icons.
- Verify the monochrome notification mark on light and dark status bars.
- Verify PWA install name, icon, maskable icon, background, and theme color.
- Verify Android and iOS launch surfaces do not flash white.
- Run mobile and desktop browser screenshots plus Android real-device or
  emulator screenshots.
- Increment Android `versionCode` for every user-visible APK release.
