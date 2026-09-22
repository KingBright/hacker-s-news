import type { Metadata, Viewport } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "FreshLoop",
  description: "让重要信息，进入你的循环",
  manifest: "/manifest.json",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  userScalable: false,
  themeColor: "#050a07",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN" className="dark" translate="no">
      <head>
        <meta name="google" content="notranslate" />
      </head>
      <body className="antialiased selection:bg-primary selection:text-black notranslate">{children}</body>
    </html>
  );
}
