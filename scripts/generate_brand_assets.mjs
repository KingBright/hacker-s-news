import fs from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(scriptDir, "..");
const requireFromFrontend = createRequire(
  path.join(rootDir, "frontend", "package.json"),
);
const sharp = requireFromFrontend("sharp");

const sourcePath = (name) => path.join(rootDir, "brand", name);
const outputPath = (...parts) => path.join(rootDir, ...parts);

const markSvg = await fs.readFile(sourcePath("freshloop-mark.svg"));
const appIconSvg = await fs.readFile(sourcePath("freshloop-app-icon.svg"));
const statusSvg = await fs.readFile(sourcePath("freshloop-status.svg"));
const monochromeSvg = Buffer.from(
  markSvg.toString("utf8").replaceAll("#19E66B", "#FFFFFF"),
);

async function ensureParent(file) {
  await fs.mkdir(path.dirname(file), { recursive: true });
}

async function render(svg, file, size, options = {}) {
  await ensureParent(file);
  let pipeline = sharp(svg, { density: 512 }).resize(size, size, {
    fit: "contain",
  });
  if (options.flatten) {
    pipeline = pipeline.flatten({ background: options.flatten });
  }
  await pipeline.png({ compressionLevel: 9 }).toFile(file);
}

async function copy(source, destination) {
  await ensureParent(destination);
  await fs.copyFile(source, destination);
}

await copy(
  sourcePath("freshloop-mark.svg"),
  outputPath("frontend", "public", "brand", "freshloop-mark.svg"),
);

await Promise.all([
  render(appIconSvg, outputPath("frontend", "public", "logo.png"), 1024),
  render(appIconSvg, outputPath("frontend", "app", "icon.png"), 1024),
  render(appIconSvg, outputPath("frontend", "public", "icon-192.png"), 192),
  render(appIconSvg, outputPath("frontend", "public", "icon-512.png"), 512),
  render(
    appIconSvg,
    outputPath("frontend", "public", "icon-maskable-512.png"),
    512,
  ),
  render(
    markSvg,
    outputPath(
      "android_client",
      "assets",
      "brand",
      "freshloop-mark.png",
    ),
    256,
  ),
  render(
    appIconSvg,
    outputPath(
      "android_client",
      "assets",
      "brand",
      "freshloop-app-icon.png",
    ),
    1024,
  ),
  render(
    markSvg,
    outputPath(
      "android_client",
      "assets",
      "brand",
      "freshloop-app-foreground.png",
    ),
    1024,
  ),
  render(
    monochromeSvg,
    outputPath(
      "android_client",
      "assets",
      "brand",
      "freshloop-app-monochrome.png",
    ),
    1024,
  ),
  render(appIconSvg, outputPath("android_client", "assets", "icon.png"), 192),
  render(
    markSvg,
    outputPath(
      "android_client",
      "android",
      "app",
      "src",
      "main",
      "res",
      "drawable-nodpi",
      "freshloop_launch_mark.png",
    ),
    160,
  ),
  render(
    markSvg,
    outputPath(
      "android_client",
      "ios",
      "Runner",
      "Assets.xcassets",
      "LaunchImage.imageset",
      "LaunchImage.png",
    ),
    168,
  ),
  render(
    markSvg,
    outputPath(
      "android_client",
      "ios",
      "Runner",
      "Assets.xcassets",
      "LaunchImage.imageset",
      "LaunchImage@2x.png",
    ),
    336,
  ),
  render(
    markSvg,
    outputPath(
      "android_client",
      "ios",
      "Runner",
      "Assets.xcassets",
      "LaunchImage.imageset",
      "LaunchImage@3x.png",
    ),
    504,
  ),
]);

const iosAppIconDir = outputPath(
  "android_client",
  "ios",
  "Runner",
  "Assets.xcassets",
  "AppIcon.appiconset",
);
const iosAppIcons = [
  { size: "20x20", idiom: "iphone", filename: "Icon-App-20x20@2x.png", scale: "2x", pixels: 40 },
  { size: "20x20", idiom: "iphone", filename: "Icon-App-20x20@3x.png", scale: "3x", pixels: 60 },
  { size: "29x29", idiom: "iphone", filename: "Icon-App-29x29@1x.png", scale: "1x", pixels: 29 },
  { size: "29x29", idiom: "iphone", filename: "Icon-App-29x29@2x.png", scale: "2x", pixels: 58 },
  { size: "29x29", idiom: "iphone", filename: "Icon-App-29x29@3x.png", scale: "3x", pixels: 87 },
  { size: "40x40", idiom: "iphone", filename: "Icon-App-40x40@2x.png", scale: "2x", pixels: 80 },
  { size: "40x40", idiom: "iphone", filename: "Icon-App-40x40@3x.png", scale: "3x", pixels: 120 },
  { size: "57x57", idiom: "iphone", filename: "Icon-App-57x57@1x.png", scale: "1x", pixels: 57 },
  { size: "57x57", idiom: "iphone", filename: "Icon-App-57x57@2x.png", scale: "2x", pixels: 114 },
  { size: "60x60", idiom: "iphone", filename: "Icon-App-60x60@2x.png", scale: "2x", pixels: 120 },
  { size: "60x60", idiom: "iphone", filename: "Icon-App-60x60@3x.png", scale: "3x", pixels: 180 },
  { size: "20x20", idiom: "ipad", filename: "Icon-App-20x20@1x.png", scale: "1x", pixels: 20 },
  { size: "20x20", idiom: "ipad", filename: "Icon-App-20x20@2x.png", scale: "2x", pixels: 40 },
  { size: "29x29", idiom: "ipad", filename: "Icon-App-29x29@1x.png", scale: "1x", pixels: 29 },
  { size: "29x29", idiom: "ipad", filename: "Icon-App-29x29@2x.png", scale: "2x", pixels: 58 },
  { size: "40x40", idiom: "ipad", filename: "Icon-App-40x40@1x.png", scale: "1x", pixels: 40 },
  { size: "40x40", idiom: "ipad", filename: "Icon-App-40x40@2x.png", scale: "2x", pixels: 80 },
  { size: "50x50", idiom: "ipad", filename: "Icon-App-50x50@1x.png", scale: "1x", pixels: 50 },
  { size: "50x50", idiom: "ipad", filename: "Icon-App-50x50@2x.png", scale: "2x", pixels: 100 },
  { size: "72x72", idiom: "ipad", filename: "Icon-App-72x72@1x.png", scale: "1x", pixels: 72 },
  { size: "72x72", idiom: "ipad", filename: "Icon-App-72x72@2x.png", scale: "2x", pixels: 144 },
  { size: "76x76", idiom: "ipad", filename: "Icon-App-76x76@1x.png", scale: "1x", pixels: 76 },
  { size: "76x76", idiom: "ipad", filename: "Icon-App-76x76@2x.png", scale: "2x", pixels: 152 },
  { size: "83.5x83.5", idiom: "ipad", filename: "Icon-App-83.5x83.5@2x.png", scale: "2x", pixels: 167 },
  { size: "1024x1024", idiom: "ios-marketing", filename: "Icon-App-1024x1024@1x.png", scale: "1x", pixels: 1024 },
];

await Promise.all(
  iosAppIcons.map(({ filename, pixels }) =>
    render(appIconSvg, path.join(iosAppIconDir, filename), pixels),
  ),
);
await fs.writeFile(
  path.join(iosAppIconDir, "Contents.json"),
  `${JSON.stringify(
    {
      images: iosAppIcons.map(({ size, idiom, filename, scale }) => ({
        size,
        idiom,
        filename,
        scale,
      })),
      info: { version: 1, author: "xcode" },
    },
    null,
    2,
  )}\n`,
);

const notificationSizes = {
  mdpi: 24,
  hdpi: 36,
  xhdpi: 48,
  xxhdpi: 72,
  xxxhdpi: 96,
};

await Promise.all(
  Object.entries(notificationSizes).map(([density, size]) =>
    render(
      statusSvg,
      outputPath(
        "android_client",
        "android",
        "app",
        "src",
        "main",
        "res",
        `drawable-${density}`,
        "ic_stat_freshloop.png",
      ),
      size,
    ),
  ),
);

console.log("FreshLoop brand assets generated.");
