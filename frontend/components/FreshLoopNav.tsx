"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const tabs = [
  { href: "/", label: "Radio", compactLabel: "Radio", icon: "radio" },
  { href: "/feed", label: "Reading", compactLabel: "Reading", icon: "menu_book" },
  { href: "/loop", label: "Loop", compactLabel: "Loop", icon: "repeat" },
  { href: "/focus", label: "Focus", compactLabel: "Focus", icon: "adjust" },
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
                ? "flex min-w-0 items-center justify-center gap-1 overflow-hidden rounded-lg bg-primary px-0.5 py-2 text-center text-[9px] font-black leading-none whitespace-nowrap text-black sm:gap-1.5 sm:px-3 sm:text-sm"
                : "flex min-w-0 items-center justify-center gap-1 overflow-hidden rounded-lg px-0.5 py-2 text-center text-[9px] font-black leading-none whitespace-nowrap text-white/70 hover:bg-white/10 hover:text-white sm:gap-1.5 sm:px-3 sm:text-sm"
            }
          >
            <span className={`material-symbols-outlined !text-[15px] sm:!text-[17px] ${selected ? "filled" : ""}`} aria-hidden="true">
              {tab.icon}
            </span>
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
