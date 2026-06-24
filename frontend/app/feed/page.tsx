"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import Image from "next/image";
import { FreshLoopNav } from "../../components/FreshLoopNav";
import { focusBucketLabel, saveLoopDraft, type WhyRecommended } from "../../src/loop";
import { buildDayPlaylists } from "../../src/day-playlists";

type ReadingMode = "original" | "compressed";
type DocumentVariant = ReadingMode | "weekly";

type MarkdownBlock =
  | { type: "heading"; level: 1 | 2 | 3; text: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "quote"; text: string }
  | { type: "code"; code: string; language?: string }
  | { type: "image"; src: string; alt: string }
  | { type: "rule" };

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

function decodeHtmlEntities(text: string) {
  return text
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'");
}

function stripUnsupportedHtml(text: string) {
  return decodeHtmlEntities(
    text
      .replace(/<br\s*\/?>/gi, "\n")
      .replace(/<\/p>/gi, "\n\n")
      .replace(/<[^>]+>/g, " "),
  );
}

function removeOrphanDollarMarkers(text: string) {
  let cleaned = "";
  let index = 0;

  while (index < text.length) {
    if (text[index] === "$") {
      let end = index + 1;
      let digitCount = 0;
      while (end < text.length && digitCount < 2 && /[0-9]/.test(text[end])) {
        end += 1;
        digitCount += 1;
      }

      if (digitCount > 0) {
        const next = text[end];
        const nextIsMoney = Boolean(next && /[0-9,.]/.test(next));
        if (!nextIsMoney) {
          const previous = cleaned.at(-1);
          const previousIsInline = Boolean(previous && !/\s/.test(previous) && previous !== "$");
          const previousIsBoundary = !previous || (!/[A-Za-z0-9_]/.test(previous) && previous !== "$");
          let nextSignificant = end;
          while (nextSignificant < text.length && /\s/.test(text[nextSignificant])) {
            nextSignificant += 1;
          }
          const followedByTerminal =
            nextSignificant >= text.length || /[)\]}。！？；，、,.;:!?]/.test(text[nextSignificant]);

          if (previousIsInline || (previousIsBoundary && followedByTerminal)) {
            index = end;
            continue;
          }
        }
      }
    }

    cleaned += text[index];
    index += 1;
  }

  return cleaned.replace(/[ \t]{2,}/g, " ");
}

function tightenTypography(text: string) {
  return removeOrphanDollarMarkers(text)
    .replace(/\u00a0/g, " ")
    .replace(/[ \t]+/g, " ")
    .replace(/\s+([,.;:!?])/g, "$1")
    .replace(/\s*([，。！？：；、])/g, "$1")
    .replace(/([（【《“‘])\s+/g, "$1")
    .replace(/\s+([）】》”’])/g, "$1")
    .replace(/([\u3400-\u9fff])\s+([\u3400-\u9fff])/g, "$1$2")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function normalizeParagraphText(text: string) {
  return tightenTypography(stripUnsupportedHtml(text));
}

function looksLikeCollapsedOriginal(text: string) {
  const trimmed = text.trim();
  if (trimmed.length < 900) return false;
  if (/\n\n/.test(trimmed)) return false;
  return !/(^|\n)\s*(#{1,3}\s|[-*+]\s|\d+[.)]\s|>\s|```|!\[|<img\b)/m.test(trimmed);
}

function splitIntoReaderParagraphs(text: string, variant: DocumentVariant) {
  const normalized = normalizeParagraphText(text);
  if (variant !== "original" || normalized.length < 460) {
    return [normalized];
  }

  const sentences = normalized
    .split(/(?<=[。！？!?])\s+|(?<=\.)\s+(?=[A-Z0-9"'([{])/)
    .map((sentence) => sentence.trim())
    .filter(Boolean);

  if (sentences.length < 3) {
    return [normalized];
  }

  const chunks: string[] = [];
  let buffer: string[] = [];
  let charCount = 0;

  for (const sentence of sentences) {
    buffer.push(sentence);
    charCount += sentence.length;
    if (charCount >= 260 || buffer.length >= 3) {
      chunks.push(normalizeParagraphText(buffer.join(" ")));
      buffer = [];
      charCount = 0;
    }
  }

  if (buffer.length > 0) {
    chunks.push(normalizeParagraphText(buffer.join(" ")));
  }

  return chunks.length > 0 ? chunks : [normalized];
}

function parseHtmlImageTag(line: string) {
  const tagMatch = line.match(/<img\b[^>]*>/i);
  if (!tagMatch) return null;
  const tag = tagMatch[0];
  const src = tag.match(/\bsrc=["']([^"']+)["']/i)?.[1];
  if (!src) return null;
  const alt = tag.match(/\balt=["']([^"']*)["']/i)?.[1] || "";
  return { src, alt: decodeHtmlEntities(alt) };
}

function parseMarkdownImage(line: string) {
  const match = line.trim().match(/^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$/);
  if (!match) return null;
  return { src: match[2], alt: decodeHtmlEntities(match[1]) };
}

function normalizeMarkdownSource(markdown: string, variant: DocumentVariant) {
  let normalized = markdown.replace(/\r\n/g, "\n").trim();
  normalized = normalized.replace(/<br\s*\/?>/gi, "\n");
  if (variant === "original" && looksLikeCollapsedOriginal(normalized)) {
    normalized = splitIntoReaderParagraphs(normalized, "original").join("\n\n");
  }
  return normalized;
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
        <strong key={`strong-${match.index}`} className="font-black text-white">
          {token.slice(2, -2)}
        </strong>,
      );
    } else if (token.startsWith("`")) {
      nodes.push(
        <code key={`code-${match.index}`} className="reader-inline-code">
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

function parseMarkdownBlocks(markdown: string, variant: DocumentVariant): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const lines = normalizeMarkdownSource(markdown, variant).split("\n");
  let list: string[] = [];
  let listOrdered = false;
  let paragraph: string[] = [];
  let inCodeBlock = false;
  let codeLines: string[] = [];
  let codeLanguage = "";

  const flushList = () => {
    if (list.length === 0) return;
    blocks.push({
      type: "list",
      ordered: listOrdered,
      items: list.map((item) => normalizeParagraphText(item)).filter(Boolean),
    });
    list = [];
  };

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    const text = normalizeParagraphText(paragraph.join(" "));
    paragraph = [];
    if (!text) return;
    for (const chunk of splitIntoReaderParagraphs(text, variant)) {
      blocks.push({ type: "paragraph", text: chunk });
    }
  };

  const flushAll = () => {
    flushParagraph();
    flushList();
  };

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    const text = line.trim();

    if (inCodeBlock) {
      if (text.startsWith("```")) {
        blocks.push({
          type: "code",
          code: codeLines.join("\n").replace(/\n+$/g, ""),
          language: codeLanguage || undefined,
        });
        inCodeBlock = false;
        codeLines = [];
        codeLanguage = "";
      } else {
        codeLines.push(rawLine.replace(/\t/g, "  "));
      }
      continue;
    }

    const codeFence = text.match(/^```([\w-]+)?\s*$/);
    if (codeFence) {
      flushAll();
      inCodeBlock = true;
      codeLanguage = codeFence[1] || "";
      continue;
    }

    if (!text) {
      flushAll();
      continue;
    }

    if (/^(-{3,}|_{3,}|\*{3,})$/.test(text)) {
      flushAll();
      blocks.push({ type: "rule" });
      continue;
    }

    const image = parseMarkdownImage(text) || parseHtmlImageTag(text);
    if (image) {
      flushAll();
      blocks.push({ type: "image", ...image });
      continue;
    }

    if (text.startsWith("### ")) {
      flushAll();
      blocks.push({ type: "heading", level: 3, text: normalizeParagraphText(text.slice(4)) });
      continue;
    }
    if (text.startsWith("## ")) {
      flushAll();
      blocks.push({ type: "heading", level: 2, text: normalizeParagraphText(text.slice(3)) });
      continue;
    }
    if (text.startsWith("# ")) {
      flushAll();
      blocks.push({ type: "heading", level: 1, text: normalizeParagraphText(text.slice(2)) });
      continue;
    }
    if (text.startsWith("> ")) {
      flushAll();
      blocks.push({ type: "quote", text: normalizeParagraphText(text.slice(2)) });
      continue;
    }

    const unordered = text.match(/^[-*+]\s+(.+)$/);
    if (unordered) {
      flushParagraph();
      if (list.length > 0 && listOrdered) flushList();
      listOrdered = false;
      list.push(unordered[1]);
      continue;
    }

    const ordered = text.match(/^\d+[.)]\s+(.+)$/);
    if (ordered) {
      flushParagraph();
      if (list.length > 0 && !listOrdered) flushList();
      listOrdered = true;
      list.push(ordered[1]);
      continue;
    }

    flushList();
    paragraph.push(text);
  }

  if (inCodeBlock && codeLines.length > 0) {
    blocks.push({
      type: "code",
      code: codeLines.join("\n").replace(/\n+$/g, ""),
      language: codeLanguage || undefined,
    });
  }

  flushAll();
  return blocks.filter((block) => block.type !== "list" || block.items.length > 0);
}

function renderDocument(blocks: MarkdownBlock[], variant: DocumentVariant) {
  return (
    <article className={`reader-doc ${variant === "original" ? "reader-doc-original" : "reader-doc-compressed"}`}>
      {blocks.map((block, index) => {
        if (block.type === "heading") {
          const Tag = `h${Math.min(block.level, 3)}` as "h1" | "h2" | "h3";
          return (
            <Tag key={`heading-${index}`} className="reader-block reader-heading" data-level={block.level}>
              {renderInlineMarkdown(block.text)}
            </Tag>
          );
        }

        if (block.type === "paragraph") {
          return (
            <p key={`paragraph-${index}`} className="reader-block reader-paragraph">
              {renderInlineMarkdown(block.text)}
            </p>
          );
        }

        if (block.type === "quote") {
          return (
            <blockquote key={`quote-${index}`} className="reader-block reader-quote">
              {renderInlineMarkdown(block.text)}
            </blockquote>
          );
        }

        if (block.type === "code") {
          return (
            <pre key={`code-${index}`} className="reader-block reader-code">
              <code>{block.code}</code>
            </pre>
          );
        }

        if (block.type === "image") {
          return (
            <figure key={`image-${index}`} className="reader-block reader-image">
              <div className="reader-image-frame">
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img src={block.src} alt={block.alt || ""} loading="lazy" />
              </div>
              {block.alt ? <figcaption className="reader-caption">{block.alt}</figcaption> : null}
            </figure>
          );
        }

        if (block.type === "rule") {
          return <div key={`rule-${index}`} className="reader-block reader-divider" />;
        }

        return (
          <div key={`list-${index}`} className="reader-block reader-list">
            {block.items.map((item, itemIndex) => (
              <div key={`item-${index}-${itemIndex}`} className="reader-list-item">
                <div className="reader-list-marker">{block.ordered ? `${itemIndex + 1}.` : "•"}</div>
                <div className="reader-list-content">{renderInlineMarkdown(item)}</div>
              </div>
            ))}
          </div>
        );
      })}
    </article>
  );
}

function WhyBalanceStrip({ why }: { why: WhyRecommended }) {
  const segments = [
    { label: "近期", value: why.balance.active_pct, className: "bg-primary" },
    { label: "长期", value: why.balance.stable_pct, className: "bg-[#93c8a8]" },
    { label: "探索", value: why.balance.explore_pct, className: "bg-white/45" },
  ];

  return (
    <div className="mt-3">
      <div className="flex h-1.5 overflow-hidden rounded-full bg-white/10">
        {segments.map((segment) => (
          <div
            key={segment.label}
            className={segment.className}
            style={{ width: `${segment.value}%` }}
            title={`${segment.label} ${segment.value}%`}
          />
        ))}
      </div>
      <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] font-semibold text-white/42">
        {segments.map((segment) => (
          <span key={segment.label}>
            <span className="text-white/70">{segment.value}%</span> {segment.label}
          </span>
        ))}
      </div>
    </div>
  );
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
  const [audioQueueIds, setAudioQueueIds] = useState<string[]>([]);
  const [audioQueueLabel, setAudioQueueLabel] = useState("");
  const [audioProgress, setAudioProgress] = useState(0);
  const [audioDuration, setAudioDuration] = useState(0);
  const [why, setWhy] = useState<WhyRecommended | null>(null);
  const [whyLoading, setWhyLoading] = useState(false);

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
  const weeklyBlocks = useMemo(() => parseMarkdownBlocks(weeklyText, "weekly"), [weeklyText]);
  const readingBlocks = useMemo(() => parseMarkdownBlocks(readingText, mode), [readingText, mode]);
  const articleDayGroups = useMemo(
    () =>
      buildDayPlaylists<FeedItem>({
        items,
        getId: (item) => item.id,
        getTimestampMs: (item) => ((item.publish_time || 0) * 1000),
        isPlayable: (item) => Boolean(item.audio_url),
        getDurationSec: (item) => item.duration_sec || 0,
        dayOrder: "desc",
        itemOrder: "desc",
        playbackOrder: "asc",
      }),
    [items],
  );

  const articleAudioById = useMemo(() => {
    const next = new Map<string, FeedItem>();
    for (const item of items) {
      if (item.audio_url) next.set(item.id, item);
    }
    return next;
  }, [items]);

  const weeklyAudioById = useMemo(() => {
    const next = new Map<string, WeeklyDigest>();
    for (const weekly of weeklies) {
      if (weekly.audio_url) next.set(weekly.id, weekly);
    }
    return next;
  }, [weeklies]);

  const selectedDayGroup = useMemo(
    () =>
      selectedItem
        ? articleDayGroups.find((group) => group.itemIds.includes(selectedItem.id)) || null
        : null,
    [articleDayGroups, selectedItem],
  );

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
      const headers = user ? { "x-user-id": user.id } : undefined;
      const [itemsRes, weekliesRes] = await Promise.all([
        fetch("/api/feed/items?product_line=curated_feed&limit=40", { headers }),
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
  }, [user]);

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
    if (!selectedItem || !user) {
      setWhy(null);
      setWhyLoading(false);
      return;
    }

    let cancelled = false;
    setWhyLoading(true);
    fetch(`/api/feed/items/${selectedItem.id}/why`, {
      headers: { "x-user-id": user.id },
    })
      .then((res) => (res.ok ? res.json() : null))
      .then((data: WhyRecommended | null) => {
        if (!cancelled) setWhy(data);
      })
      .finally(() => {
        if (!cancelled) setWhyLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [selectedItem, user]);

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

  const stopAudioQueue = useCallback((reset = true) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.pause();
    setIsAudioPlaying(false);
    if (!reset) return;
    setActiveAudioId(null);
    setAudioTitle("");
    setAudioQueueIds([]);
    setAudioQueueLabel("");
    setAudioProgress(0);
    setAudioDuration(0);
  }, []);

  const playQueueFromId = useCallback(async (queueIds: string[], startId: string, label: string) => {
    const audio = audioRef.current;
    const article = articleAudioById.get(startId);
    const weekly = weeklyAudioById.get(startId);
    const url = article?.audio_url || weekly?.audio_url;
    const title = article ? cleanTitle(article.title) : weekly?.title || "";

    if (!audio || !url) return;

    setAudioQueueIds(queueIds);
    setAudioQueueLabel(label);
    setActiveAudioId(startId);
    setAudioTitle(title);
    setAudioProgress(0);
    setAudioDuration(article?.duration_sec || weekly?.duration_sec || 0);
    audio.src = url;
    audio.load();

    try {
      await audio.play();
      setIsAudioPlaying(true);
    } catch {
      setIsAudioPlaying(false);
    }
  }, [articleAudioById, weeklyAudioById]);

  const toggleAudioPlayback = useCallback(async (id: string, queueIds: string[], label: string) => {
    const audio = audioRef.current;
    if (!audio) return;

    if (activeAudioId === id) {
      if (audio.paused) {
        try {
          await audio.play();
          setIsAudioPlaying(true);
        } catch {
          setIsAudioPlaying(false);
        }
      } else {
        audio.pause();
        setIsAudioPlaying(false);
      }
      return;
    }

    await playQueueFromId(queueIds, id, label);
  }, [activeAudioId, playQueueFromId]);

  const playArticleDay = useCallback(
    async (dayTitle: string, queueIds: string[], startId?: string) => {
      const playableQueue = queueIds.filter((id) => articleAudioById.has(id));
      const targetId = startId && playableQueue.includes(startId) ? startId : playableQueue[0];
      if (!targetId) return;
      await playQueueFromId(playableQueue, targetId, `${dayTitle} · 阅读日播放列表`);
    },
    [articleAudioById, playQueueFromId],
  );

  const playNextAudio = useCallback(async () => {
    if (!activeAudioId) return;
    const currentIndex = audioQueueIds.indexOf(activeAudioId);
    const nextId = currentIndex >= 0 ? audioQueueIds[currentIndex + 1] : null;
    if (!nextId) {
      stopAudioQueue();
      return;
    }
    await playQueueFromId(audioQueueIds, nextId, audioQueueLabel);
  }, [activeAudioId, audioQueueIds, audioQueueLabel, playQueueFromId, stopAudioQueue]);

  const playPreviousAudio = useCallback(async () => {
    if (!activeAudioId) return;
    const currentIndex = audioQueueIds.indexOf(activeAudioId);
    if (currentIndex <= 0) return;
    await playQueueFromId(audioQueueIds, audioQueueIds[currentIndex - 1], audioQueueLabel);
  }, [activeAudioId, audioQueueIds, audioQueueLabel, playQueueFromId]);

  const sendToLoop = useCallback((item: FeedItem) => {
    saveLoopDraft({
      feedbackMode: "balance",
      title: cleanTitle(item.title),
      references: [
        {
          sourceType: "article",
          sourceId: item.id,
          sourceUrl: item.original_url,
          title: cleanTitle(item.title),
          quoteText: item.subtitle || undefined,
        },
      ],
    });
    window.location.href = "/loop";
  }, []);

  return (
    <div className="relative min-h-screen overflow-x-hidden bg-background-dark text-white font-display">
      <audio
        ref={audioRef}
        onPause={() => setIsAudioPlaying(false)}
        onPlaying={() => setIsAudioPlaying(true)}
        onTimeUpdate={() => setAudioProgress(audioRef.current?.currentTime || 0)}
        onLoadedMetadata={() => setAudioDuration(audioRef.current?.duration || 0)}
        onEnded={() => {
          void playNextAudio();
        }}
        className="hidden"
      />

      <header className="sticky top-0 z-30 border-b border-white/5 bg-background-dark/95 px-4 pb-4 pt-12 backdrop-blur-md">
        <div className="mx-auto flex max-w-6xl flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div className="flex items-center gap-3">
            <Image src="/logo.png" alt="FreshLoop" width={40} height={40} className="rounded-xl shadow-lg ring-1 ring-white/10" />
            <div>
              <div className="text-xl font-bold leading-none tracking-tight text-white">FreshLoop</div>
              <div className="mt-1 text-[10px] font-bold uppercase tracking-[0.18em] text-[#93c8a8]">
                Curated Reading
              </div>
            </div>
          </div>
          <div className="w-full md:max-w-md">
            <FreshLoopNav />
          </div>
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
              {articleDayGroups.map((group) => (
                <div key={group.key} className="rounded-xl border border-white/6 bg-black/15 p-3">
                  <div className="mb-3 flex items-start justify-between gap-3">
                    <div>
                      <div className="text-sm font-black text-white">{group.title}</div>
                      <div className="mt-1 text-[11px] text-white/45">
                        {group.items.length} 篇 · {group.playableCount} 段可播
                      </div>
                    </div>
                    {group.playbackIds.length > 0 ? (
                      <button
                        onClick={() => void playArticleDay(group.title, group.playbackIds)}
                        className="rounded-full bg-white/10 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-white/15"
                      >
                        播放当天
                      </button>
                    ) : null}
                  </div>
                  <div className="space-y-2">
                    {group.items.map((item) => {
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
                              {item.audio_url ? (
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
                  </div>
                </div>
              ))}
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
                          void toggleAudioPlayback(weekly.id, [weekly.id], "Weekly Brief");
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
                      onClick={() => void toggleAudioPlayback(selectedWeekly.id, [selectedWeekly.id], "Weekly Brief")}
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
                  <div className="mx-auto max-w-4xl pb-24">
                    <div className="rounded-[28px] border border-white/7 bg-[linear-gradient(180deg,rgba(15,31,24,0.95),rgba(10,17,14,0.98))] px-5 py-6 shadow-[0_28px_90px_rgba(0,0,0,0.28)] md:px-8 md:py-8">
                      <div className="mb-5 flex flex-wrap items-center gap-2 text-[11px] font-black uppercase tracking-[0.18em] text-[#93c8a8]">
                        <span>Weekly Brief</span>
                        <span>·</span>
                        <span>交叉主题梳理</span>
                      </div>
                      {renderDocument(weeklyBlocks, "weekly")}
                    </div>
                  </div>
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
                      onClick={() => void toggleAudioPlayback(selectedItem.id, [selectedItem.id], "单篇收听")}
                      className="flex h-11 shrink-0 items-center justify-center gap-2 rounded-full bg-primary px-4 text-sm font-black text-black hover:bg-primary/90"
                    >
                      <span className="material-symbols-outlined text-[21px]">
                        {activeAudioId === selectedItem.id && isAudioPlaying ? "pause" : "play_arrow"}
                      </span>
                      {formatDuration(selectedItem.duration_sec)}
                    </button>
                  ) : null}
                  {selectedDayGroup && selectedDayGroup.playbackIds.length > 1 ? (
                    <button
                      onClick={() => void playArticleDay(selectedDayGroup.title, selectedDayGroup.playbackIds, selectedItem.id)}
                      className="flex h-11 shrink-0 items-center justify-center gap-2 rounded-full border border-white/10 bg-white/5 px-4 text-sm font-black text-white hover:border-primary/40 hover:text-primary"
                    >
                      <span className="material-symbols-outlined text-[21px]">queue_music</span>
                      播放{selectedDayGroup.shortTitle}
                    </button>
                  ) : null}
                </div>
                {selectedItem.subtitle ? (
                  <p className="mt-3 max-w-2xl text-sm leading-6 text-white/50">{selectedItem.subtitle}</p>
                ) : null}
                {why ? (
                  <div className="mt-4 rounded-2xl border border-primary/20 bg-primary/8 p-4">
                    <div className="flex flex-wrap items-center gap-2 text-xs font-black uppercase tracking-[0.18em] text-primary">
                      <span>Why</span>
                      <span>{focusBucketLabel(why.bucket)}</span>
                      <span>{why.score}</span>
                    </div>
                    <WhyBalanceStrip why={why} />
                    <div className="mt-3 space-y-2 text-sm leading-7 text-white/68">
                      {why.reasons.map((reason) => (
                        <div key={reason}>{reason}</div>
                      ))}
                    </div>
                  </div>
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
                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      onClick={() => sendToLoop(selectedItem)}
                      className="flex h-10 items-center gap-2 rounded-full bg-primary px-3 text-sm font-black text-black hover:bg-primary/90"
                    >
                      <span className="material-symbols-outlined text-[18px]">format_quote</span>
                      Loop
                    </button>
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
                    {whyLoading ? (
                      <div className="flex h-10 items-center gap-2 rounded-full bg-white/5 px-3 text-sm font-bold text-white/45 ring-1 ring-white/10">
                        <div className="size-4 rounded-full border-2 border-white/20 border-t-primary animate-spin" />
                        Why
                      </div>
                    ) : null}
                  </div>
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
                  <div className="mx-auto max-w-4xl pb-24">
                    <div
                      className={`rounded-[28px] border px-5 py-6 shadow-[0_28px_90px_rgba(0,0,0,0.26)] md:px-8 md:py-8 ${
                        mode === "original"
                          ? "border-white/8 bg-[linear-gradient(180deg,rgba(25,35,29,0.98),rgba(14,20,17,0.98))]"
                          : "border-primary/10 bg-[linear-gradient(180deg,rgba(13,21,17,0.98),rgba(9,14,11,0.98))]"
                      }`}
                    >
                      <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
                        <div className="flex flex-wrap items-center gap-2 text-[11px] font-black uppercase tracking-[0.18em] text-[#93c8a8]">
                          <span>{mode === "original" ? "原文整理版" : "干货压缩"}</span>
                          {selectedItem.source_name ? (
                            <>
                              <span>·</span>
                              <span>{selectedItem.source_name}</span>
                            </>
                          ) : null}
                        </div>
                        {selectedItem.original_url ? (
                          <a
                            href={selectedItem.original_url}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex h-9 items-center gap-2 rounded-full bg-white/6 px-3 text-sm font-bold text-white/72 ring-1 ring-white/10 transition hover:bg-white/10 hover:text-white"
                          >
                            <span className="material-symbols-outlined text-[17px]">open_in_new</span>
                            原文链接
                          </a>
                        ) : null}
                      </div>
                      {renderDocument(readingBlocks, mode)}
                    </div>
                  </div>
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
          <div className="mx-auto max-w-6xl">
            <div className="mb-3 h-1.5 overflow-hidden rounded-full bg-white/10">
              <div
                className="h-full rounded-full bg-primary transition-all"
                style={{ width: `${audioDuration > 0 ? Math.min((audioProgress / audioDuration) * 100, 100) : 0}%` }}
              />
            </div>
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="truncate text-sm font-black text-white">{audioTitle}</div>
                <div className="mt-1 text-xs font-medium text-[#93c8a8]">
                  {audioQueueLabel || "FreshLoop Listening"}
                </div>
                <div className="mt-1 text-[11px] text-white/35">
                  {formatDuration(Math.floor(audioProgress))} / {formatDuration(Math.floor(audioDuration))}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => void playPreviousAudio()}
                  disabled={audioQueueIds.indexOf(activeAudioId) <= 0}
                  className="flex size-10 shrink-0 items-center justify-center rounded-full bg-white/8 text-white transition disabled:cursor-not-allowed disabled:opacity-30"
                >
                  <span className="material-symbols-outlined text-[22px]">skip_previous</span>
                </button>
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
                <button
                  onClick={() => void playNextAudio()}
                  disabled={audioQueueIds.indexOf(activeAudioId) < 0 || audioQueueIds.indexOf(activeAudioId) >= audioQueueIds.length - 1}
                  className="flex size-10 shrink-0 items-center justify-center rounded-full bg-white/8 text-white transition disabled:cursor-not-allowed disabled:opacity-30"
                >
                  <span className="material-symbols-outlined text-[22px]">skip_next</span>
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
