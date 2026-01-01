"use client";

import { useEffect, useState, useRef, useCallback } from 'react';
import { Item } from '../src/types';

function formatTime(seconds: number): string {
  if (!seconds || isNaN(seconds)) return "00:00";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
}

export default function Home() {
  const [items, setItems] = useState<Item[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isDragging, setIsDragging] = useState(false);

  // Audio ref
  const audioRef = useRef<HTMLAudioElement>(null);

  // Poll for new items
  const fetchItems = useCallback(() => {
    fetch('/api/items')
      .then(res => res.json())
      .then(data => {
        setItems(data);
      })
      .catch(err => console.error('Failed to fetch items:', err));
  }, []);

  useEffect(() => {
    fetchItems();
    const interval = setInterval(fetchItems, 30000);
    return () => clearInterval(interval);
  }, [fetchItems]);

  // Audio Event Handlers
  const togglePlay = () => {
    if (audioRef.current) {
      if (isPlaying) {
        audioRef.current.pause();
      } else {
        audioRef.current.play();
      }
      setIsPlaying(!isPlaying);
    }
  };

  const playItem = (id: string, url: string) => {
    if (currentId === id) {
      togglePlay();
      return;
    }

    setCurrentId(id);
    if (audioRef.current) {
      audioRef.current.src = url;
      audioRef.current.play()
        .then(() => setIsPlaying(true))
        .catch(e => console.error("Play failed", e));
    }
  };

  const playNext = useCallback(() => {
    if (!currentId) return;
    const currentIndex = items.findIndex(i => i.id === currentId);
    if (currentIndex !== -1 && currentIndex < items.length - 1) {
      const nextItem = items.slice(currentIndex + 1).find(i => i.audio_url);
      if (nextItem && nextItem.audio_url) {
        playItem(nextItem.id, nextItem.audio_url);
      }
    }
  }, [currentId, items]);

  const playPrev = useCallback(() => {
    if (!currentId) return;
    const currentIndex = items.findIndex(i => i.id === currentId);
    if (currentIndex > 0) {
      const prevItem = items.slice(0, currentIndex).reverse().find(i => i.audio_url);
      if (prevItem && prevItem.audio_url) {
        playItem(prevItem.id, prevItem.audio_url);
      }
    }
  }, [currentId, items]);

  const handleTimeUpdate = () => {
    if (audioRef.current && !isDragging) {
      setProgress(audioRef.current.currentTime);
    }
  };

  const handleLoadedMetadata = () => {
    if (audioRef.current) {
      setDuration(audioRef.current.duration);
    }
  };

  // Circular Progress Calculation
  const circleRadius = 22;
  const circumference = 2 * Math.PI * circleRadius; // ~138
  const progressPercent = duration > 0 ? (progress / duration) : 0;
  const strokeDashoffset = circumference - (progressPercent * circumference);

  const currentItem = items.find(i => i.id === currentId);
  const today = new Date().toLocaleDateString('en-US', { weekday: 'long', month: 'short', day: 'numeric' });

  return (
    <div className="relative flex h-full min-h-screen w-full flex-col overflow-x-hidden max-w-md mx-auto shadow-2xl pb-32 bg-background-light dark:bg-background-dark text-slate-900 dark:text-white font-display">
      <audio
        ref={audioRef}
        onTimeUpdate={handleTimeUpdate}
        onLoadedMetadata={handleLoadedMetadata}
        onEnded={playNext}
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        className="hidden"
      />

      {/* Header */}
      <header className="sticky top-0 z-20 bg-background-light/95 dark:bg-background-dark/95 backdrop-blur-md px-4 pt-12 pb-4 border-b border-black/5 dark:border-white/5">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[28px] font-bold leading-none tracking-tight text-slate-900 dark:text-white">FreshLoop</h1>
            <p className="text-xs text-slate-500 dark:text-[#93c8a8] mt-1 font-medium tracking-wide uppercase">Audio Briefing • Zen Mode</p>
          </div>
        </div>
      </header>

      <main className="flex flex-col gap-6 p-4">
        {/* Hero Card: Daily Summary */}
        <section className="relative overflow-hidden rounded-3xl bg-surface-dark shadow-lg ring-1 ring-white/5 group">
          {/* Abstract Background Pattern */}
          <div className="absolute inset-0 opacity-40 mix-blend-overlay" style={{ backgroundImage: 'radial-gradient(circle at 80% 20%, rgba(25, 230, 107, 0.3) 0%, transparent 50%)' }}></div>
          <div className="relative flex flex-col p-6 z-10">
            <div className="flex items-start justify-between mb-8">
              <div>
                <p className="text-[#93c8a8] text-sm font-medium uppercase tracking-wider mb-1">{today}</p>
                <h2 className="text-3xl font-bold text-white tracking-tight leading-none">Good Morning</h2>
              </div>
              <div className="h-10 w-10 rounded-full bg-primary/20 flex items-center justify-center text-primary">
                <span className="material-symbols-outlined">wb_sunny</span>
              </div>
            </div>
            <div className="space-y-4">
              <div className="flex items-end gap-3">
                <span className="text-5xl font-bold text-primary font-display tabular-nums">{items.length}</span>
                <span className="text-lg text-white/80 font-medium mb-1.5">Fresh stories tailored for you</span>
              </div>
              <div className="w-full bg-black/20 h-1.5 rounded-full overflow-hidden">
                <div className="bg-primary h-full w-[35%] rounded-full shadow-[0_0_10px_rgba(25,230,107,0.5)]"></div>
              </div>
              <div className="flex items-center justify-between text-sm text-[#93c8a8]">
                <span className="flex items-center gap-1.5">
                  <span className="material-symbols-outlined text-[16px]">schedule</span>
                  Updated just now
                </span>
                <button onClick={() => items.length > 0 && items[0].audio_url && playItem(items[0].id, items[0].audio_url)} className="text-primary font-bold hover:underline decoration-2 underline-offset-4 flex items-center gap-1">
                  Play Digest <span className="material-symbols-outlined text-[16px]">arrow_forward</span>
                </button>
              </div>
            </div>
          </div>
        </section>

        {/* Collections Section (Items List) */}
        <section className="flex flex-col gap-4">
          <div className="flex items-center justify-between px-1">
            <h3 className="text-xl font-bold text-slate-900 dark:text-white">Your Feed</h3>
            <button className="text-sm font-medium text-slate-500 dark:text-[#93c8a8] hover:text-primary transition-colors">Edit</button>
          </div>

          <div className="flex flex-col gap-3">
            {items.map((item, index) => {
              const isActive = currentId === item.id;
              return (
                <div
                  key={item.id}
                  onClick={() => item.audio_url && playItem(item.id, item.audio_url)}
                  className={`
                    group flex items-center gap-4 bg-white dark:bg-surface-dark p-4 rounded-2xl ring-1 shadow-sm hover:shadow-md transition-all cursor-pointer active:scale-[0.99]
                    ${isActive ? 'ring-primary dark:ring-primary' : 'ring-black/5 dark:ring-white/5 hover:ring-primary/50 dark:hover:ring-primary/50'}
                  `}
                >
                  <div className="relative shrink-0">
                    <div className={`flex items-center justify-center rounded-xl size-14 shadow-inner ${isActive ? 'bg-primary text-black' : 'bg-blue-100 dark:bg-[#244732] text-blue-600 dark:text-white'}`}>
                      {/* Use a generic icon or map based on content if possible */}
                      <span className="material-symbols-outlined text-[28px]">graphic_eq</span>
                    </div>
                    {isActive && (
                      <div className="absolute -bottom-1 -right-1 bg-surface-dark rounded-full p-0.5">
                        <div className="size-4 rounded-full bg-primary border-2 border-surface-dark animate-pulse"></div>
                      </div>
                    )}
                  </div>
                  <div className="flex flex-col justify-center grow min-w-0">
                    <h4 className={`text-base font-bold leading-tight truncate ${isActive ? 'text-primary' : 'text-slate-900 dark:text-white'}`}>
                      {item.title}
                    </h4>
                    <p className="text-slate-500 dark:text-[#93c8a8] text-sm mt-1 line-clamp-1">
                      {item.summary || "Audio briefing available"}
                    </p>
                    <div className="flex items-center gap-2 mt-1.5">
                      <span className="text-xs font-medium text-slate-400 dark:text-white/40 bg-slate-100 dark:bg-black/20 px-2 py-0.5 rounded">
                        {item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : 'Now'}
                      </span>
                    </div>
                  </div>
                  <div className="shrink-0">
                    <button className={`flex items-center justify-center size-10 rounded-full transition-colors ${isActive && isPlaying ? 'bg-primary text-black' : 'bg-slate-100 dark:bg-black/20 text-slate-900 dark:text-white group-hover:bg-primary group-hover:text-black'}`}>
                      <span className="material-symbols-outlined filled text-[24px]">
                        {isActive && isPlaying ? 'pause' : 'play_arrow'}
                      </span>
                    </button>
                  </div>
                </div>
              );
            })}

            {items.length === 0 && (
              <div className="text-center py-20 text-slate-500 dark:text-[#93c8a8]">
                Loading stories...
              </div>
            )}
          </div>
        </section>

        {/* Secondary Feature Card (Static for now) */}
        <section className="mt-2">
          <div className="rounded-2xl bg-gradient-to-r from-indigo-900 to-indigo-800 p-5 text-white shadow-lg relative overflow-hidden">
            <div className="absolute right-0 top-0 h-full w-1/2 opacity-20" style={{ backgroundImage: 'radial-gradient(circle at 100% 0%, #ffffff 0%, transparent 70%)' }}></div>
            <div className="relative z-10 flex items-center justify-between">
              <div>
                <h4 className="font-bold text-lg">Weekly Deep Dive</h4>
                <p className="text-indigo-200 text-sm mt-1">This week: The Future of Energy</p>
              </div>
              <button className="bg-white/10 hover:bg-white/20 p-2 rounded-full backdrop-blur-sm transition-colors">
                <span className="material-symbols-outlined">bookmark_add</span>
              </button>
            </div>
          </div>
        </section>
      </main>

      {/* Persistent Player Bar (Floating) */}
      {currentItem && (
        <div className="fixed bottom-[88px] left-0 right-0 px-4 z-40 max-w-md mx-auto">
          <div className="bg-[#1e1e1e] dark:bg-black rounded-2xl p-3 pr-4 shadow-[0_8px_30px_rgb(0,0,0,0.4)] ring-1 ring-white/10 flex items-center gap-3 backdrop-blur-xl">
            {/* Art / Progress Circle */}
            <div className="relative size-12 shrink-0 flex items-center justify-center">
              <svg className="transform -rotate-90 size-12 drop-shadow-[0_0_8px_rgba(25,230,107,0.3)]">
                <circle className="text-white/10" cx="24" cy="24" fill="transparent" r={circleRadius} stroke="currentColor" strokeWidth="2"></circle>
                <circle
                  className="text-primary transition-all duration-300"
                  cx="24" cy="24"
                  fill="transparent"
                  r={circleRadius}
                  stroke="currentColor"
                  strokeDasharray={circumference}
                  strokeDashoffset={strokeDashoffset}
                  strokeLinecap="round"
                  strokeWidth="2"
                ></circle>
              </svg>
              <div className="absolute inset-0 m-auto size-8 rounded-full bg-surface-highlight overflow-hidden flex items-center justify-center">
                {/* Fallback pattern or image if available */}
                <span className="material-symbols-outlined text-white/50 text-sm">music_note</span>
              </div>
            </div>

            <div className="flex flex-col grow overflow-hidden">
              <div className="flex items-center gap-2">
                <span className="bg-primary/20 text-primary text-[10px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wider">Live</span>
                <p className="text-white text-sm font-bold truncate">{currentItem.title}</p>
              </div>
              <p className="text-[#93c8a8] text-xs truncate">
                {formatTime(duration - progress)} remaining
              </p>
            </div>

            <div className="flex items-center gap-1">
              <button onClick={togglePlay} className="text-white hover:text-primary p-2 rounded-full transition-colors">
                <span className="material-symbols-outlined text-[28px] filled">
                  {isPlaying ? 'pause_circle' : 'play_circle'}
                </span>
              </button>
              <button onClick={playNext} className="text-white/60 hover:text-white p-2 rounded-full transition-colors">
                <span className="material-symbols-outlined text-[26px]">skip_next</span>
              </button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}

