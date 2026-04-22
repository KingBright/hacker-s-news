import type { Metadata, Viewport } from "next";
import { Work_Sans } from "next/font/google";
import "./globals.css";

const workSans = Work_Sans({
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700", "800"],
  variable: "--font-work-sans",
});

export const metadata: Metadata = {
  title: "FreshLoop",
  description: "Zen Reading",
  manifest: "/manifest.json",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  userScalable: false,
  themeColor: "#1c1917",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN" className={`dark ${workSans.variable}`} translate="no">
      <head>
        <meta name="google" content="notranslate" />
      </head>
      <body className="antialiased selection:bg-primary selection:text-black notranslate">{children}</body>
    </html>
  );
}
