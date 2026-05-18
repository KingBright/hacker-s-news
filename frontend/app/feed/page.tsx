"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import Image from "next/image";
import Link from "next/link";

type ReadingMode = "original" | "compressed";

interface FeedItem {
  id: string;
  product_line: string;
  item_type: string;
  primary_mode: string;
  title: string;
  subtitle?: string | null;
  source_name?: string | null;
  source_url?: string | null;
  original_url?: string | null;
  canonical_url?: string | null;
  publish_time?: number | null;
  has_audio?: boolean | null;
  audio_url?: string | null;
  duration_sec?: number | null;
  reading_time_min?: number | null;
  quality_score?: number | null;
  tags?: string | null;
  status?: string | null;
}

interface FeedItemContent {
  item_id: string;
  original_html?: string | null;
  reader_markdown?: string | null;
  plain_text?: string | null;
  compressed_markdown?: string | null;
  audio_script?: string | null;
  key_points_json?: string | null;
}

interface WeeklyDigest {
  id: string;
  feed_item_id?: string | null;
  week_start: number;
  week_end: number;
  title: string;
  digest_markdown?: string | null;
  audio_script?: string | null;
  audio_url?: string | null;
  duration_sec?: number | null;
  included_item_ids_json?: string | null;
  themes_json?: string | null;
  status?: string | null;
}

interface StoredUser {
  id: string;
  username: string;
}

function formatDate(ts?: number | null) {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleDateString("zh-CN", {
    month: "short",
    day: "numeric",
  });
}

function formatDuration(seconds?: number | null) {
  if (!seconds) return "音频";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

function parseJsonList(value?: string | null): string[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed.filter((item) => typeof item === "string") : [];
  } catch {
    return [];
  }
}

function cleanTitle(title: string) {
  return title.replace(/^【.*?】/, "").trim();
}

function contentForMode(content: FeedItemContent | null, mode: ReadingMode) {
  if (!content) return "";
  if (mode === "compressed") {
    return (
      content.compressed_markdown ||
      content.audio_script ||
      parseJsonList(content.key_points_json)
        .map((point) => `- ${point}`)
        .join("\n") ||
      content.plain_text ||
      "这篇文章还没有生成干货压缩。"
    );
  }
  return content.reader_markdown || content.plain_text || "";
}

function renderInlineMarkdown(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > cursor) nodes.push(text.slice(cursor, match.index));
    const token = match[0];
    if (token.startsWith("**")) {
      nodes.push(
        <strong key={`strong-${match.index}`} className="font-black text-white/90">
          {token.slice(2, -2)}
        </strong>,
      );
    } else if (token.startsWith("`")) {
      nodes.push(
        <code key={`code-${match.index}`} className="rounded bg-white/10 px-1.5 py-0.5 text-[0.92em] text-[#bff6d2]">
          {token.slice(1, -1)}
        </code>,
      );
    } else {
      const link = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token);
      nodes.push(
        link ? (
          <a key={`link-${match.index}`} href={link[2]} target="_blank" rel="noreferrer" className="text-primary underline decoration-primary/35 underline-offset-4">
            {link[1]}
          </a>
        ) : (
          token
        ),
      );
    }
    cursor = match.index + token.length;
  }

  if (cursor < text.length) nodes.push(text.slice(cursor));
  return nodes;
}

function renderMarkdown(markdown: string) {
  const lines = markdown.split("\n");
  const nodes: ReactNode[] = [];
  let list: string[] = [];
  let listKind: "ul" | "ol" = "ul";
  let paragraph: string[] = [];

  const flushList = () => {
    if (list.length === 0) return;
    const items = list;
    const kind = listKind;
    list = [];
    const className = "my-5 space-y-2 pl-5 text-[15px] leading-7 text-white/76";
    const children = items.map((item, index) => (
      <li key={`${item}-${index}`} className={kind === "ul" ? "list-disc pl-1" : "list-decimal pl-1"}>
        {renderInlineMarkdown(item)}
      </li>
    ));
    nodes.push(kind === "ul" ? (
      <ul key={`list-${nodes.length}`} className={className}>{children}</ul>
    ) : (
      <ol key={`list-${nodes.length}`} className={className}>{children}</ol>
    ));
  };

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    const text = paragraph.join("\n").trim();
    paragraph = [];
    if (!text) return;
    nodes.push(
      <p key={`p-${nodes.length}`} className="my-5 whitespace-pre-line text-[16px] leading-8 text-white/76">
        {renderInlineMarkdown(text)}
      </p>,
    );
  };

  const flushAll = () => {
    flushParagraph();
    flushList();
  };

  lines.forEach((line, index) => {
    const text = line.trim();
    if (!text) {
      flushAll();
      return;
    }
    if (text.startsWith("### ")) {
      flushAll();
      nodes.push(
        <h3 key={index} className="mt-8 text-lg font-black leading-tight text-white">
          {renderInlineMarkdown(text.slice(4))}
        </h3>,
      );
      return;
    }
    if (text.startsWith("## ")) {
      flushAll();
      nodes.push(
        <h2 key={index} className="mt-9 text-xl font-black leading-tight text-white">
          {renderInlineMarkdown(text.slice(3))}
        </h2>,
      );
      return;
    }
    if (text.startsWith("# ")) {
      flushAll();
      nodes.push(
        <h1 key={index} className="mt-8 text-2xl font-black leading-tight text-white">
          {renderInlineMarkdown(text.slice(2))}
        </h1>,
      );
      return;
    }
    if (/^[-*+]\s+/.test(text)) {
      flushParagraph();
      if (list.length > 0 && listKind !== "ul") flushList();
      listKind = "ul";
      list.push(text.slice(2));
      return;
    }
    const ordered = /^(\d+)[.)]\s+(.+)$/.exec(text);
    if (ordered) {
      flushParagraph();
      if (list.length > 0 && listKind !== "ol") flushList();
      listKind = "ol";
      list.push(ordered[2]);
      return;
    }
    if (text.startsWith("> ")) {
      flushAll();
      nodes.push(
        <blockquote key={index} className="my-6 border-l-2 border-primary/60 pl-4 text-[15px] leading-7 text-white/68">
          {renderInlineMarkdown(text.slice(2))}
        </blockquote>,
      );
      return;
    }
    flushList();
    paragraph.push(text);
  });

  flushAll();
  return nodes;
}

export default function FeedPage() {
  const [user, setUser] = useState<StoredUser | null>(null);
  const [items, setItems] = useState<FeedItem[]>([]);
  const [weeklies, setWeeklies] = useState<WeeklyDigest[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedWeeklyId, setSelectedWeeklyId] = useState<string | null>(null);
  const [content, setContent] = useState<FeedItemContent | null>(null);
  const [mode, setMode] = useState<ReadingMode>("original");
  const [loading, setLoading] = useState(true);
  const [contentLoading, setContentLoading] = useState(false);
  const [activeAudioId, setActiveAudioId] = useState<string | null>(null);
  const [isAudioPlaying, setIsAudioPlaying] = useState(false);
  const [audioTitle, setAudioTitle] = useState("");

  const readerRef = useRef<HTMLDivElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const progressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const restoredProgressKey = useRef<string | null>(null);

  const selectedItem = useMemo(
    () => (selectedWeeklyId ? null : items.find((item) => item.id === selectedId) || null),
    [items, selectedId, selectedWeeklyId],
  );

  const selectedWeekly = useMemo(
    () => weeklies.find((weekly) => weekly.id === selectedWeeklyId) || null,
    [selectedWeeklyId, weeklies],
  );

  const weeklyText = useMemo(
    () => selectedWeekly?.digest_markdown || selectedWeekly?.audio_script || "",
    [selectedWeekly],
  );

  const readingText = useMemo(() => contentForMode(content, mode), [content, mode]);

  useEffect(() => {
    const storedUser = localStorage.getItem("freshloop_user");
    if (!storedUser) return;
    try {
      setUser(JSON.parse(storedUser));
    } catch {
      setUser(null);
    }
  }, []);

  const loadFeed = useCallback(async () => {
    setLoading(true);
    try {
      const [itemsRes, weekliesRes] = await Promise.all([
        fetch("/api/feed/items?product_line=curated_feed&limit=40"),
        fetch("/api/feed/weeklies"),
      ]);
      const nextItems: FeedItem[] = itemsRes.ok ? await itemsRes.json() : [];
      const nextWeeklies: WeeklyDigest[] = weekliesRes.ok ? await weekliesRes.json() : [];
      setItems(nextItems.filter((item) => item.item_type === "article"));
      setWeeklies(nextWeeklies);
      setSelectedId((current) => current || nextItems.find((item) => item.item_type === "article")?.id || null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadFeed();
  }, [loadFeed]);

  useEffect(() => {
    if (!selectedId || selectedWeeklyId) {
      setContent(null);
      return;
    }

    let cancelled = false;
    setContentLoading(true);
    fetch(`/api/feed/items/${selectedId}/content`)
      .then((res) => (res.ok ? res.json() : null))
      .then((data: FeedItemContent | null) => {
        if (!cancelled) setContent(data);
      })
      .finally(() => {
        if (!cancelled) setContentLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [selectedId, selectedWeeklyId]);

  useEffect(() => {
    if (!selectedId || selectedWeeklyId || !readingText || !readerRef.current) return;
    const key = `freshloop_reader_${selectedId}_${mode}`;
    if (restoredProgressKey.current === key) return;
    restoredProgressKey.current = key;

    requestAnimationFrame(() => {
      const ratio = Number(localStorage.getItem(key) || "0");
      const el = readerRef.current;
      if (!el || !Number.isFinite(ratio) || ratio <= 0) return;
      el.scrollTop = (el.scrollHeight - el.clientHeight) * Math.min(ratio, 0.95);
    });
  }, [mode, readingText, selectedId, selectedWeeklyId]);

  const saveReadingProgress = useCallback(() => {
    if (!selectedId || selectedWeeklyId || !readerRef.current) return;
    const el = readerRef.current;
    const denominator = el.scrollHeight - el.clientHeight;
    const ratio = denominator <= 0 ? 0 : el.scrollTop / denominator;
    const key = `freshloop_reader_${selectedId}_${mode}`;
    localStorage.setItem(key, String(Math.max(0, Math.min(ratio, 1))));

    if (progressTimer.current) clearTimeout(progressTimer.current);
    progressTimer.current = setTimeout(() => {
      if (!user) return;
      fetch(`/api/feed/items/${selectedId}/progress`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-user-id": user.id,
        },
        body: JSON.stringify({ mode, scroll_ratio: ratio }),
      }).catch(() => undefined);
    }, 800);
  }, [mode, selectedId, selectedWeeklyId, user]);

  const playAudio = useCallback((id: string, title: string, url?: string | null) => {
    if (!url || !audioRef.current) return;
    const audio = audioRef.current;

    if (activeAudioId === id && !audio.paused) {
      audio.pause();
      setIsAudioPlaying(false);
      return;
    }

    if (activeAudioId !== id) {
      audio.src = url;
      audio.load();
      setActiveAudioId(id);
      setAudioTitle(title);
    }

    audio
      .play()
      .then(() => setIsAudioPlaying(true))
      .catch(() => setIsAudioPlaying(false));
  }, [activeAudioId]);

  return (
    <div className="relative min-h-screen overflow-x-hidden bg-background-dark text-white font-display">
      <audio
        ref={audioRef}
        onPause={() => setIsAudioPlaying(false)}
        onPlaying={() => setIsAudioPlaying(true)}
        onEnded={() => setIsAudioPlaying(false)}
        className="hidden"
      />

      <header className="sticky top-0 z-30 border-b border-white/5 bg-background-dark/95 px-4 pb-4 pt-12 backdrop-blur-md">
        <div className="mx-auto flex max-w-6xl items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <Image src="/logo.png" alt="FreshLoop" width={40} height={40} className="rounded-xl shadow-lg ring-1 ring-white/10" />
            <div>
              <div className="text-xl font-bold leading-none tracking-tight text-white">FreshLoop</div>
              <div className="mt-1 text-[10px] font-bold uppercase tracking-[0.18em] text-[#93c8a8]">
                Curated Reading
              </div>
            </div>
          </div>
          <nav className="flex rounded-lg bg-white/5 p-1 ring-1 ring-white/10">
            <Link href="/" className="rounded-md px-3 py-2 text-sm font-bold text-white/60 hover:bg-white/10 hover:text-white">
              Radio
            </Link>
            <Link href="/feed" className="rounded-md bg-primary px-3 py-2 text-sm font-bold text-black">
              Reading
            </Link>
          </nav>
        </div>
      </header>

      <main className="mx-auto grid max-w-6xl gap-4 px-4 py-5 lg:grid-cols-[340px_minmax(0,1fr)]">
        <aside className="space-y-4">
          <section className="rounded-2xl bg-surface-dark p-4 shadow-lg ring-1 ring-white/5">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-black uppercase tracking-[0.16em] text-[#93c8a8]">Daily</h2>
              <button
                onClick={() => void loadFeed()}
                className="flex size-9 items-center justify-center rounded-full bg-white/10 text-white hover:bg-white/20 disabled:opacity-50"
                title="Refresh"
              >
                <span className={`material-symbols-outlined text-[20px] ${loading ? "animate-spin" : ""}`}>refresh</span>
              </button>
            </div>
            <div className="mt-4 space-y-2">
              {items.map((item) => {
                const selected = item.id === selectedId;
                return (
                  <button
                    key={item.id}
                    onClick={() => {
                      restoredProgressKey.current = null;
                      setSelectedWeeklyId(null);
                      setSelectedId(item.id);
                    }}
                    className={`w-full rounded-lg border p-3 text-left transition ${
                      selected
                        ? "border-primary bg-primary/10 text-white"
                        : "border-white/5 bg-black/20 text-white hover:border-primary/50"
                    }`}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <h3 className="line-clamp-2 text-sm font-black leading-snug">{cleanTitle(item.title)}</h3>
                      <span className={selected ? "text-primary" : "text-white/45"}>
                        {item.has_audio ? (
                          <span className="material-symbols-outlined text-[18px]">headphones</span>
                        ) : (
                          <span className="material-symbols-outlined text-[18px]">article</span>
                        )}
                      </span>
                    </div>
                    <div className={`mt-3 flex items-center gap-2 text-xs ${selected ? "text-[#93c8a8]" : "text-white/45"}`}>
                      <span>{formatDate(item.publish_time)}</span>
                      <span>·</span>
                      <span>{item.reading_time_min || 1} min read</span>
                      {item.quality_score ? (
                        <>
                          <span>·</span>
                          <span>{item.quality_score}/10</span>
                        </>
                      ) : null}
                    </div>
                  </button>
                );
              })}
              {items.length === 0 && !loading ? (
                <div className="rounded-lg border border-dashed border-white/10 p-6 text-center text-sm text-white/45">
                  暂无精选文章
                </div>
              ) : null}
            </div>
          </section>

          <section className="rounded-2xl bg-[#102b36] p-4 shadow-lg ring-1 ring-white/5">
            <h2 className="text-sm font-black uppercase tracking-[0.16em] text-[#93c8a8]">Weekly Brief</h2>
            <div className="mt-4 space-y-2">
              {weeklies.map((weekly) => (
                <div
                  key={weekly.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    restoredProgressKey.current = null;
                    setSelectedId(null);
                    setSelectedWeeklyId(weekly.id);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      restoredProgressKey.current = null;
                      setSelectedId(null);
                      setSelectedWeeklyId(weekly.id);
                    }
                  }}
                  className={`w-full cursor-pointer rounded-lg border p-3 text-left transition ${
                    weekly.id === selectedWeeklyId
                      ? "border-primary bg-primary/10"
                      : "border-white/5 bg-white/5 hover:border-primary/50"
                  }`}
                >
                  <div className="flex items-center justify-between gap-3">
                    <h3 className="line-clamp-2 text-sm font-black leading-snug text-white">{weekly.title}</h3>
                    {weekly.audio_url ? (
                      <button
                        onClick={(event) => {
                          event.stopPropagation();
                          playAudio(weekly.id, weekly.title, weekly.audio_url);
                        }}
                        className="flex size-8 shrink-0 items-center justify-center rounded-full bg-primary text-black"
                        title="Play weekly audio"
                      >
                        <span className="material-symbols-outlined text-[22px]">
                          {activeAudioId === weekly.id && isAudioPlaying ? "pause" : "play_arrow"}
                        </span>
                      </button>
                    ) : null}
                  </div>
                  <div className="mt-2 text-xs font-medium text-white/50">
                    {formatDate(weekly.week_start)} - {formatDate(weekly.week_end)}
                    {weekly.audio_url ? ` · ${formatDuration(weekly.duration_sec)}` : " · 文稿"}
                  </div>
                </div>
              ))}
              {weeklies.length === 0 ? (
                <div className="rounded-lg border border-dashed border-white/10 p-4 text-center text-sm text-white/45">
                  周汇总文稿生成后会出现在这里
                </div>
              ) : null}
            </div>
          </section>
        </aside>

        <section className="min-h-[calc(100vh-120px)] overflow-hidden rounded-2xl bg-surface-dark shadow-lg ring-1 ring-white/5">
          {selectedWeekly ? (
            <div className="flex h-full flex-col">
              <div className="border-b border-white/5 p-5">
                <div className="flex flex-wrap items-center gap-2 text-xs font-bold uppercase tracking-[0.16em] text-[#93c8a8]">
                  <span>FreshLoop Weekly</span>
                  <span>·</span>
                  <span>
                    {formatDate(selectedWeekly.week_start)} - {formatDate(selectedWeekly.week_end)}
                  </span>
                  {selectedWeekly.audio_url ? (
                    <>
                      <span>·</span>
                      <span>{formatDuration(selectedWeekly.duration_sec)}</span>
                    </>
                  ) : null}
                </div>
                <div className="mt-3 flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
                  <h1 className="max-w-3xl text-2xl font-black leading-tight text-white md:text-4xl">
                    {selectedWeekly.title}
                  </h1>
                  {selectedWeekly.audio_url ? (
                    <button
                      onClick={() => playAudio(selectedWeekly.id, selectedWeekly.title, selectedWeekly.audio_url)}
                      className="flex h-11 shrink-0 items-center justify-center gap-2 rounded-full bg-primary px-4 text-sm font-black text-black hover:bg-primary/90"
                    >
                      <span className="material-symbols-outlined text-[21px]">
                        {activeAudioId === selectedWeekly.id && isAudioPlaying ? "pause" : "play_arrow"}
                      </span>
                      {formatDuration(selectedWeekly.duration_sec)}
                    </button>
                  ) : null}
                </div>
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto px-5 py-6 md:px-10">
                {weeklyText ? (
                  <article className="mx-auto max-w-3xl pb-24">{renderMarkdown(weeklyText)}</article>
                ) : (
                  <div className="mx-auto max-w-2xl rounded-lg border border-dashed border-white/10 p-10 text-center text-white/45">
                    周汇总文稿还在生成中
                  </div>
                )}
              </div>
            </div>
          ) : selectedItem ? (
            <div className="flex h-full flex-col">
              <div className="border-b border-white/5 p-5">
                <div className="flex flex-wrap items-center gap-2 text-xs font-bold uppercase tracking-[0.16em] text-[#93c8a8]">
                  <span>{selectedItem.source_name || "FreshLoop"}</span>
                  <span>·</span>
                  <span>{formatDate(selectedItem.publish_time)}</span>
                  {selectedItem.reading_time_min ? (
                    <>
                      <span>·</span>
                      <span>{selectedItem.reading_time_min} min</span>
                    </>
                  ) : null}
                </div>
                <div className="mt-3 flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
                  <h1 className="max-w-3xl text-2xl font-black leading-tight text-white md:text-4xl">
                    {cleanTitle(selectedItem.title)}
                  </h1>
                  {selectedItem.audio_url ? (
                    <button
                      onClick={() => playAudio(selectedItem.id, selectedItem.title, selectedItem.audio_url)}
                      className="flex h-11 shrink-0 items-center justify-center gap-2 rounded-full bg-primary px-4 text-sm font-black text-black hover:bg-primary/90"
                    >
                      <span className="material-symbols-outlined text-[21px]">
                        {activeAudioId === selectedItem.id && isAudioPlaying ? "pause" : "play_arrow"}
                      </span>
                      {formatDuration(selectedItem.duration_sec)}
                    </button>
                  ) : null}
                </div>
                {selectedItem.subtitle ? (
                  <p className="mt-3 max-w-2xl text-sm leading-6 text-white/50">{selectedItem.subtitle}</p>
                ) : null}
                <div className="mt-5 flex flex-wrap items-center justify-between gap-3">
                  <div className="flex rounded-lg bg-black/25 p-1 ring-1 ring-white/10">
                    <button
                      onClick={() => {
                        restoredProgressKey.current = null;
                        setMode("original");
                      }}
                      className={`rounded-md px-3 py-2 text-sm font-black ${
                        mode === "original" ? "bg-surface-highlight text-white shadow-sm" : "text-white/50"
                      }`}
                    >
                      原版
                    </button>
                    <button
                      onClick={() => {
                        restoredProgressKey.current = null;
                        setMode("compressed");
                      }}
                      className={`rounded-md px-3 py-2 text-sm font-black ${
                        mode === "compressed" ? "bg-surface-highlight text-white shadow-sm" : "text-white/50"
                      }`}
                    >
                      干货压缩
                    </button>
                  </div>
                  {selectedItem.original_url ? (
                    <a
                      href={selectedItem.original_url}
                      target="_blank"
                      rel="noreferrer"
                      className="flex h-10 items-center gap-2 rounded-full bg-white/5 px-3 text-sm font-bold text-white/65 ring-1 ring-white/10 hover:text-white"
                    >
                      <span className="material-symbols-outlined text-[18px]">open_in_new</span>
                      Source
                    </a>
                  ) : null}
                </div>
              </div>

              <div
                ref={readerRef}
                onScroll={saveReadingProgress}
                className="min-h-0 flex-1 overflow-y-auto px-5 py-6 md:px-10"
              >
                {contentLoading ? (
                  <div className="flex h-64 items-center justify-center text-white/40">
                    <div className="size-7 rounded-full border-2 border-white/20 border-t-primary animate-spin" />
                  </div>
                ) : readingText ? (
                  <article className="mx-auto max-w-3xl pb-24">{renderMarkdown(readingText)}</article>
                ) : (
                  <div className="mx-auto max-w-2xl rounded-lg border border-dashed border-white/10 p-10 text-center text-white/45">
                    内容还在生成中
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="flex h-[70vh] items-center justify-center text-white/45">
              {loading ? "加载中..." : "暂无可读内容"}
            </div>
          )}
        </section>
      </main>

      {activeAudioId ? (
        <div className="fixed inset-x-0 bottom-0 z-40 border-t border-white/10 bg-[#1e1e1e]/95 px-4 py-3 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="truncate text-sm font-black text-white">{audioTitle}</div>
              <div className="mt-1 text-xs font-medium text-[#93c8a8]">FreshLoop Listening</div>
            </div>
            <button
              onClick={() => {
                if (!audioRef.current) return;
                if (audioRef.current.paused) {
                  void audioRef.current.play();
                } else {
                  audioRef.current.pause();
                }
              }}
              className="flex size-11 shrink-0 items-center justify-center rounded-full bg-primary text-black"
            >
              <span className="material-symbols-outlined filled text-[28px]">
                {isAudioPlaying ? "pause" : "play_arrow"}
              </span>
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
