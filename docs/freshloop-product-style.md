# FreshLoop Product And Style Notes

FreshLoop should feel like one product with two modes, not two separate apps.

## Product Lines

### Radio

- Audio-first.
- Queue, background playback, notification controls, and resume position matter
  more than dense reading controls.
- Existing feed cards and Morphing Player define the mobile visual baseline.

### Reading

- Reading-first curated subscription feed.
- Supports original and compressed modes.
- Supports optional article audio.
- Weekly digest is a listen-first feed item and belongs to the broader
  FreshLoop audio ecosystem.

## Visual Baseline

Use the existing app language:

- Dark background: `#111111` / near black.
- Surface: `#1E1E1E`.
- Highlight surface: `#2A2A2A` or dark green `#244732`.
- Accent: `#19E66B`.
- Muted green text: `#93C8A8`.
- Cards: rounded, compact, content-dense, no marketing hero layout.
- Motion: subtle state changes, not decorative animation.

## Reading UI Rules

- Reading pages can use a lighter inner text surface only if it still feels
  embedded in FreshLoop. Avoid full-page bright document-reader styling.
- Keep article cards structurally close to the existing audio cards: icon block,
  title, compact metadata, small action affordance.
- Weekly audio should look like a FreshLoop audio module, not a separate podcast
  marketplace card.
- On Android, avoid changing Rust Bridge models unless the API contract truly
  requires it. Dart-side models are lower risk for new read-only product
  surfaces.

## Copy

- Keep labels short: `Radio`, `Reading`, `原版`, `干货压缩`, `周汇总`.
- Avoid explanatory in-app text that describes how the app works. Empty states
  can be concise.
