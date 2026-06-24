"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const tabs = [
  { href: "/", label: "Radio", compactLabel: "Radio" },
  { href: "/feed", label: "Reading", compactLabel: "Read" },
  { href: "/loop", label: "Loop", compactLabel: "Loop" },
  { href: "/focus", label: "Focus", compactLabel: "Focus" },
];

export function FreshLoopNav() {
  const pathname = usePathname();

  return (
    <nav className="mt-4 grid grid-cols-4 gap-1 rounded-xl bg-white/5 p-1 ring-1 ring-white/10">
      {tabs.map((tab) => {
        const selected =
          tab.href === "/" ? pathname === "/" : pathname.startsWith(tab.href);
        return (
          <Link
            key={tab.href}
            href={tab.href}
            aria-label={tab.label}
            className={
              selected
                ? "flex min-w-0 items-center justify-center overflow-hidden rounded-lg bg-primary px-1 py-2 text-center text-[10px] font-black leading-none whitespace-nowrap text-black sm:px-3 sm:text-sm"
                : "flex min-w-0 items-center justify-center overflow-hidden rounded-lg px-1 py-2 text-center text-[10px] font-black leading-none whitespace-nowrap text-white/70 hover:bg-white/10 hover:text-white sm:px-3 sm:text-sm"
            }
          >
            <span className="block min-w-0 truncate sm:hidden">
              {tab.compactLabel}
            </span>
            <span className="hidden min-w-0 truncate sm:block">{tab.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}
