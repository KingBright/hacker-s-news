"use client";

import Image from "next/image";
import Link from "next/link";
import type { ReactNode } from "react";
import { FreshLoopNav } from "./FreshLoopNav";

interface BrandHeaderProps {
  actions?: ReactNode;
  contentClassName?: string;
  onBrandClick?: () => void;
}

function BrandIdentity({ onClick }: { onClick?: () => void }) {
  const content = (
    <>
      <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-white/5 ring-1 ring-white/10">
        <Image
          src="/brand/freshloop-mark.svg"
          alt=""
          width={34}
          height={34}
          priority
        />
      </span>
      <span className="text-xl font-black leading-none tracking-[-0.035em] text-white sm:text-2xl">
        FreshLoop
      </span>
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 items-center gap-3 text-left"
        aria-label="FreshLoop"
      >
        {content}
      </button>
    );
  }

  return (
    <Link href="/" className="flex min-w-0 items-center gap-3" aria-label="FreshLoop 首页">
      {content}
    </Link>
  );
}

export function BrandHeader({
  actions,
  contentClassName = "max-w-4xl",
  onBrandClick,
}: BrandHeaderProps) {
  return (
    <header className="sticky top-0 z-30 border-b border-white/5 bg-background-dark/95 px-4 pb-4 pt-[calc(env(safe-area-inset-top)+1rem)] backdrop-blur-md">
      <div className={`mx-auto flex w-full items-center justify-between gap-4 ${contentClassName}`}>
        <BrandIdentity onClick={onBrandClick} />
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </div>
      <div className={`mx-auto w-full ${contentClassName}`}>
        <FreshLoopNav />
      </div>
    </header>
  );
}
