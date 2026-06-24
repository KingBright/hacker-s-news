"use client";

import { useEffect, useState } from "react";
import Image from "next/image";
import { FreshLoopNav } from "../../components/FreshLoopNav";
import { LoginModal } from "../../components/LoginModal";
import { focusKindLabel, type BalanceRule, type FocusCard, type FocusSummary } from "../../src/loop";

interface StoredUser {
  id: string;
  username: string;
}

function FocusSection({
  title,
  detail,
  items,
}: {
  title: string;
  detail: string;
  items: FocusCard[];
}) {
  return (
    <section className="rounded-3xl bg-surface-dark p-5 shadow-lg ring-1 ring-white/6">
      <div>
        <h2 className="text-xl font-black text-white">{title}</h2>
        <p className="mt-1 text-sm text-white/55">{detail}</p>
      </div>
      <div className="mt-5 flex flex-wrap gap-3">
        {items.length === 0 && (
          <div className="rounded-2xl border border-dashed border-white/12 bg-black/20 px-4 py-4 text-sm leading-7 text-white/45">
            还没有足够信号。去 Loop 里表达几次后，这里会逐渐出现可校准的关注结构。
          </div>
        )}
        {items.map((item) => (
          <div
            key={`${title}-${item.label}`}
            className="min-w-[220px] flex-1 rounded-2xl border border-white/8 bg-black/20 p-4"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="text-sm font-black text-white">{item.label}</div>
              <div className="rounded-full bg-primary/12 px-2.5 py-1 text-xs font-black text-primary">
                {item.score}
              </div>
            </div>
            <div className="mt-2 text-xs font-semibold uppercase tracking-[0.18em] text-white/35">
              {focusKindLabel(item.kind)}
            </div>
            <div className="mt-3 text-sm leading-7 text-white/55">{item.evidence}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

function BalanceCard({ title, balance }: { title: string; balance: BalanceRule }) {
  const segments = [
    { label: "近期表达", value: balance.active_pct, color: "bg-primary" },
    { label: "长期兴趣", value: balance.stable_pct, color: "bg-[#93c8a8]" },
    { label: "探索位", value: balance.explore_pct, color: "bg-white/45" },
  ];

  return (
    <div className="rounded-2xl border border-white/8 bg-black/20 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-black uppercase tracking-[0.2em] text-white/40">
          {title}
        </div>
        <div className="text-xs font-black text-primary">
          探索 {balance.explore_pct}%
        </div>
      </div>
      <div className="mt-3 flex h-2 overflow-hidden rounded-full bg-white/8">
        {segments.map((segment) => (
          <div
            key={segment.label}
            className={segment.color}
            style={{ width: `${segment.value}%` }}
            title={`${segment.label} ${segment.value}%`}
          />
        ))}
      </div>
      <div className="mt-3 grid grid-cols-3 gap-2 text-[11px] font-semibold text-white/48">
        {segments.map((segment) => (
          <div key={segment.label}>
            <span className="text-white/78">{segment.value}%</span> {segment.label}
          </div>
        ))}
      </div>
      <div className="mt-3 text-sm leading-7 text-white/72">{balance.note}</div>
    </div>
  );
}

export default function FocusPage() {
  const [user, setUser] = useState<StoredUser | null>(null);
  const [showLogin, setShowLogin] = useState(false);
  const [loading, setLoading] = useState(false);
  const [summary, setSummary] = useState<FocusSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const storedUser = localStorage.getItem("freshloop_user");
    if (!storedUser) return;
    try {
      const nextUser = JSON.parse(storedUser) as StoredUser;
      setUser(nextUser);
      void (async () => {
        setLoading(true);
        try {
          const res = await fetch("/api/focus", {
            headers: { "x-user-id": nextUser.id },
          });
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          const data = (await res.json()) as FocusSummary;
          setSummary(data);
        } catch (err: unknown) {
          setError(err instanceof Error ? err.message : "加载失败");
        } finally {
          setLoading(false);
        }
      })();
    } catch {
      setUser(null);
    }
  }, []);

  const handleLogin = (nextUser: StoredUser) => {
    setUser(nextUser);
    localStorage.setItem("freshloop_user", JSON.stringify(nextUser));
    window.location.reload();
  };

  const handleLogout = () => {
    setUser(null);
    localStorage.removeItem("freshloop_user");
    setSummary(null);
  };

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-6xl flex-col bg-background-dark px-4 pb-16 text-white">
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
                Attention Mix
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

      <main className="space-y-6 pt-4">
        <section className="rounded-3xl bg-surface-dark p-6 shadow-lg ring-1 ring-white/6">
          <div className="max-w-3xl">
            <h1 className="text-3xl font-black tracking-tight">Focus</h1>
            <p className="mt-3 text-sm leading-7 text-white/58">
              这里展示 FreshLoop 当前的注意力配比：近期表达会提高侧重，长期兴趣保持稳定，探索位始终保留。
            </p>
          </div>

          {loading && (
            <div className="mt-6 flex items-center gap-3 text-sm text-white/55">
              <div className="size-5 rounded-full border-2 border-white/15 border-t-primary animate-spin" />
              正在整理你的表达闭环…
            </div>
          )}

          {!user && !loading && (
            <div className="mt-6 rounded-3xl border border-dashed border-white/12 bg-black/20 p-6 text-sm leading-7 text-white/55">
              登录后你可以看到当前关注点、长期兴趣、最近降低的主题，以及内容混合比例。
            </div>
          )}

          {error && (
            <div className="mt-6 rounded-3xl border border-red-500/20 bg-red-500/10 p-4 text-sm text-red-200">
              {error}
            </div>
          )}

          {summary && (
            <div className="mt-6 grid gap-4 lg:grid-cols-3">
              <BalanceCard title="Reading Mix" balance={summary.reading_balance} />
              <BalanceCard title="Radio Mix" balance={summary.radio_balance} />
              <div className="rounded-2xl border border-white/8 bg-black/20 p-4">
                <div className="text-xs font-black uppercase tracking-[0.2em] text-white/40">
                  Loop Status
                </div>
                <div className="mt-3 space-y-1 text-sm text-white/78">
                  <div>表达次数：{summary.stats.expression_count}</div>
                  <div>已吸收：{summary.stats.processed_expression_count}</div>
                  <div>待提炼：{summary.stats.pending_expression_count}</div>
                  <div>偏好信号：{summary.stats.signal_count}</div>
                </div>
              </div>
            </div>
          )}
        </section>

        {summary && (
          <>
            <FocusSection
              title="当前焦点"
              detail="会提高接下来几轮的排序和摘要侧重点。"
              items={summary.current_focus}
            />
            <FocusSection
              title="长期兴趣"
              detail="来自反复出现的表达和较稳定的偏好。"
              items={summary.long_term_focus}
            />
            <FocusSection
              title="最近降低"
              detail="这里只表示阶段性降一点，不代表彻底不看。"
              items={summary.recently_reduced}
            />
            <FocusSection
              title="偏好来源"
              detail="你最近更容易被这些来源或出处触发表达。"
              items={summary.preferred_sources}
            />
            <FocusSection
              title="偏好形态"
              detail="近期更适合用阅读还是音频去承载。"
              items={summary.preferred_formats}
            />
          </>
        )}
      </main>
    </div>
  );
}
