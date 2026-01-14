import type { Viewport } from "next";

export const viewport: Viewport = {
    width: "device-width",
    initialScale: 1,
    userScalable: true,
};

export default function AdminLayout({
    children,
}: {
    children: React.ReactNode;
}) {
    return <>{children}</>;
}
