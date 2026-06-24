"use client";

import { useEffect, useMemo, useState } from "react";
import Image from "next/image";
import { FreshLoopNav } from "../../components/FreshLoopNav";
import { LoginModal } from "../../components/LoginModal";
import {
  clearLoopDraft,
  loadLoopDraft,
  loopPreferenceStatusLabel,
  type LoopDraft,
  type LoopFeedbackMode,
  type LoopPost,
  type LoopPostResponse,
  type LoopPostReference,
  type LoopReferenceDraft,
} from "../../src/loop";

interface StoredUser {
  id: string;
  username: string;
}

const feedbackModes: Array<{
  id: LoopFeedbackMode;
  label: string;
  hint: string;
}> = [
  { id: "balance", label: "平衡记录", hint: "保留这个方向，但不要极端偏置" },
  { id: "boost", label: "加重一点", hint: "接下来可以多给我一点这个方向" },
  { id: "reduce", label: "降一点", hint: "近期先少一点，但不要完全消失" },
  { id: "observe", label: "仅观察", hint: "先记住，不立刻明显改配比" },
];

function formatTime(ts?: number | null) {
  if (!ts) return "刚刚";
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function feedbackModeLabel(mode?: string | null) {
  return (
    feedbackModes.find((item) => item.id === mode)?.label ||
    feedbackModes[0].label
  );
}

function buildSourceRef(references: LoopReferenceDraft[]) {
  const first = references[0];
  if (!first) return null;
  if (first.sourceId) {
    return `${first.sourceType}:${first.sourceId}`;
  }
  if (first.sourceUrl) {
    return `${first.sourceType}:${first.sourceUrl}`;
  }
  return null;
}

function normalizeLoopPostResponse(payload: unknown): LoopPostResponse | null {
  if (!payload || typeof payload !== "object") return null;
  const candidate = payload as Record<string, unknown>;
  const references = Array.isArray(candidate.references)
    ? (candidate.references as LoopPostReference[])
    : [];

  if (candidate.post && typeof candidate.post === "object") {
    return {
      post: candidate.post as LoopPost,
      references,
    };
  }

  return {
    post: candidate as unknown as LoopPost,
    references,
  };
}

export default function LoopPage() {
  const [user, setUser] = useState<StoredUser | null>(null);
  const [showLogin, setShowLogin] = useState(false);
  const [posts, setPosts] = useState<LoopPostResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<LoopDraft | null>(null);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [feedbackMode, setFeedbackMode] = useState<LoopFeedbackMode>("balance");

  useEffect(() => {
    const storedUser = localStorage.getItem("freshloop_user");
    if (storedUser) {
      try {
        setUser(JSON.parse(storedUser));
      } catch {
        setUser(null);
      }
    }
    const nextDraft = loadLoopDraft();
    if (nextDraft) {
      setDraft(nextDraft);
      setTitle(nextDraft.title || "");
      setBody(nextDraft.body || "");
      setFeedbackMode(nextDraft.feedbackMode || "balance");
    }
  }, []);

  useEffect(() => {
    if (!user) {
      setPosts([]);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    fetch("/api/loop/posts?limit=60", {
      headers: { "x-user-id": user.id },
    })
      .then((res) => {
        if (!res.ok) {
          throw new Error(`HTTP ${res.status}`);
        }
        return res.json();
      })
      .then((data: unknown) => {
        if (cancelled) return;
        const normalized = Array.isArray(data)
          ? data
              .map((item) => normalizeLoopPostResponse(item))
              .filter((item): item is LoopPostResponse => item !== null)
          : [];
        setPosts(normalized);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "加载失败");
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [user]);

  const draftReferences = useMemo(() => draft?.references || [], [draft]);

  const handleLogin = (nextUser: StoredUser) => {
    setUser(nextUser);
    localStorage.setItem("freshloop_user", JSON.stringify(nextUser));
  };

  const handleLogout = () => {
    setUser(null);
    localStorage.removeItem("freshloop_user");
  };

  const publish = async () => {
    if (!user) {
      setShowLogin(true);
      return;
    }
    if (!body.trim()) {
      setError("请输入你的表达。");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const response = await fetch("/api/loop/posts", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-user-id": user.id,
        },
        body: JSON.stringify({
          post_type: draftReferences.length > 0 ? "quote_comment" : "thought",
          feedback_mode: feedbackMode,
          title: title.trim() || null,
          body: body.trim(),
          visibility: "private",
          source_ref: buildSourceRef(draftReferences),
          references: draftReferences.map((reference) => ({
            source_type: reference.sourceType,
            source_id: reference.sourceId || null,
            source_url: reference.sourceUrl || null,
            title: reference.title || null,
            quote_text: reference.quoteText || null,
            start_ms: reference.startMs || null,
            end_ms: reference.endMs || null,
          })),
        }),
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      const created = normalizeLoopPostResponse(await response.json());
      if (!created) {
        throw new Error("Unexpected Loop response");
      }
      setPosts((current) => [created, ...current]);
      setTitle("");
      setBody("");
      setFeedbackMode("balance");
      setDraft(null);
      clearLoopDraft();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "发布失败");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-4xl flex-col bg-background-dark px-4 pb-16 text-white">
      <LoginModal
        isOpen={showLogin}
        onClose={() => setShowLogin(false)}
        onLogin={handleLogin}
      />

      <header className="sticky top-0 z-20 bg-background-dark/95 px-1 pt-8 pb-4 backdrop-blur-md">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <Image
              src="/logo.png"
              alt="FreshLoop"
              width={40}
              height={40}
              className="rounded-xl shadow-lg ring-1 ring-white/10"
            />
            <div>
              <div className="text-xl font-bold leading-none tracking-tight text-white">
                FreshLoop
              </div>
              <div className="mt-1 text-xs font-medium uppercase tracking-[0.22em] text-[#93c8a8]">
                My Loop
              </div>
            </div>
          </div>
          <button
            onClick={() =>
              user ? window.confirm("退出登录？") && handleLogout() : setShowLogin(true)
            }
            className="rounded-full bg-white/6 px-4 py-2 text-sm font-semibold text-white/75 ring-1 ring-white/10 hover:bg-white/10 hover:text-white"
          >
            {user ? user.username : "登录"}
          </button>
        </div>
        <FreshLoopNav />
      </header>

      <main className="grid gap-6 pt-4 lg:grid-cols-[1.1fr_0.9fr]">
        <section className="rounded-3xl bg-surface-dark p-5 shadow-lg ring-1 ring-white/6">
          <div className="flex items-center justify-between gap-4">
            <div>
              <h1 className="text-2xl font-black tracking-tight">Loop</h1>
              <p className="mt-1 text-sm text-white/55">
                像引用转发一样写下判断。FreshLoop 会调节侧重点，但不会把内容砍成黑白两类。
              </p>
            </div>
            {draftReferences.length > 0 && (
              <button
                onClick={() => {
                  setDraft(null);
                  clearLoopDraft();
                }}
                className="rounded-full bg-white/6 px-3 py-2 text-xs font-semibold text-white/70 hover:bg-white/10 hover:text-white"
              >
                清空引用
              </button>
            )}
          </div>

          <div className="mt-5 grid gap-3 sm:grid-cols-2">
            {feedbackModes.map((mode) => (
              <button
                key={mode.id}
                onClick={() => setFeedbackMode(mode.id)}
                className={
                  feedbackMode === mode.id
                    ? "rounded-2xl border border-primary/70 bg-primary/10 p-3 text-left"
                    : "rounded-2xl border border-white/8 bg-black/20 p-3 text-left hover:border-white/20 hover:bg-white/6"
                }
              >
                <div className="text-sm font-black text-white">{mode.label}</div>
                <div className="mt-1 text-xs leading-5 text-white/55">
                  {mode.hint}
                </div>
              </button>
            ))}
          </div>

          {draftReferences.length > 0 && (
            <div className="mt-5 rounded-2xl border border-primary/20 bg-primary/8 p-4">
              <div className="text-xs font-black uppercase tracking-[0.2em] text-primary">
                引用原内容
              </div>
              <div className="mt-3 space-y-3">
                {draftReferences.map((reference, index) => (
                  <div
                    key={`${reference.sourceType}-${reference.sourceId || index}`}
                    className="rounded-2xl border border-white/8 bg-black/20 p-4"
                  >
                    <div className="text-sm font-bold text-white">
                      {reference.title || "原内容"}
                    </div>
                    {reference.quoteText && (
                      <div className="mt-2 text-sm leading-7 text-white/65">
                        “{reference.quoteText}”
                      </div>
                    )}
                    {reference.sourceUrl && (
                      <a
                        href={reference.sourceUrl}
                        target="_blank"
                        rel="noreferrer"
                        className="mt-3 inline-flex text-xs font-semibold text-[#93c8a8] underline decoration-[#93c8a8]/30 underline-offset-4"
                      >
                        查看原始链接
                      </a>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="mt-5 space-y-3">
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="标题可选，用来概括这一轮表达"
              className="w-full rounded-2xl border border-white/8 bg-black/20 px-4 py-3 text-sm text-white outline-none ring-0 placeholder:text-white/25 focus:border-primary/40"
            />
            <textarea
              value={body}
              onChange={(event) => setBody(event.target.value)}
              placeholder="直接说你的判断、偏好、边界，或者为什么想继续跟踪这条内容。"
              rows={8}
              className="w-full rounded-3xl border border-white/8 bg-black/20 px-4 py-4 text-sm leading-7 text-white outline-none ring-0 placeholder:text-white/25 focus:border-primary/40"
            />
          </div>

          {error && (
            <div className="mt-4 rounded-2xl border border-red-500/20 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              {error}
            </div>
          )}

          <div className="mt-5 flex items-center justify-between gap-4">
            <div className="text-xs leading-6 text-white/45">
              这会参与下一轮排序；长期信号整理后会出现在 Focus。
            </div>
            <button
              onClick={() => void publish()}
              disabled={submitting}
              className="rounded-full bg-primary px-5 py-3 text-sm font-black text-black transition hover:brightness-105 disabled:opacity-50"
            >
              {submitting ? "发布中..." : "发布到 Loop"}
            </button>
          </div>
        </section>

        <section className="rounded-3xl bg-surface-dark p-5 shadow-lg ring-1 ring-white/6">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-xl font-black">My Loop</h2>
              <p className="mt-1 text-sm text-white/55">
                原始表达会一直保留，偏好只是它的衍生信号。
              </p>
            </div>
            {loading && (
              <div className="size-5 rounded-full border-2 border-white/15 border-t-primary animate-spin" />
            )}
          </div>

          {!user && !loading && (
            <div className="mt-6 rounded-3xl border border-dashed border-white/12 bg-black/20 p-6 text-sm leading-7 text-white/55">
              登录后可以看到自己的表达历史，并让推荐开始跟着你的引用和判断动态变化。
            </div>
          )}

          {user && posts.length === 0 && !loading && (
            <div className="mt-6 rounded-3xl border border-dashed border-white/12 bg-black/20 p-6 text-sm leading-7 text-white/55">
              还没有 Loop 表达。你可以从 Radio 或 Reading 里引用一条内容，然后在这里补上你的判断。
            </div>
          )}

          <div className="mt-5 space-y-4">
            {posts.map(({ post, references }) => (
              <article
                key={post.id}
                className="rounded-3xl border border-white/8 bg-black/20 p-5"
              >
                <div className="flex flex-wrap items-center gap-2 text-xs">
                  <span className="rounded-full bg-primary/14 px-3 py-1 font-black text-primary">
                    {feedbackModeLabel(post.feedback_mode)}
                  </span>
                  <span className="rounded-full bg-white/6 px-3 py-1 font-semibold text-white/55">
                    {formatTime(post.created_at)}
                  </span>
                  {post.preference_status && (
                    <span className="rounded-full bg-white/6 px-3 py-1 font-semibold text-white/55">
                      {loopPreferenceStatusLabel(post.preference_status)}
                    </span>
                  )}
                </div>
                {post.title && (
                  <h3 className="mt-4 text-lg font-black leading-tight text-white">
                    {post.title}
                  </h3>
                )}
                <div className="mt-3 whitespace-pre-line text-[15px] leading-7 text-white/80">
                  {post.body}
                </div>

                {references.length > 0 && (
                  <div className="mt-4 space-y-3 border-t border-white/8 pt-4">
                    {references.map((reference) => (
                      <div
                        key={reference.id}
                        className="rounded-2xl border border-white/8 bg-white/[0.03] p-4"
                      >
                        <div className="text-sm font-bold text-white">
                          {reference.title || "原内容"}
                        </div>
                        {reference.quote_text && (
                          <div className="mt-2 text-sm leading-7 text-white/60">
                            “{reference.quote_text}”
                          </div>
                        )}
                        {reference.source_url && (
                          <a
                            href={reference.source_url}
                            target="_blank"
                            rel="noreferrer"
                            className="mt-3 inline-flex text-xs font-semibold text-[#93c8a8] underline decoration-[#93c8a8]/30 underline-offset-4"
                          >
                            打开原始链接
                          </a>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </article>
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}
