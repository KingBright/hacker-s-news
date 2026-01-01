import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "FreshLoop",
  description: "Zen Reading",
  manifest: "/manifest.json",
  viewport: "width=device-width, initial-scale=1, maximum-scale=1, user-scalable=0",
  themeColor: "#1c1917",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
