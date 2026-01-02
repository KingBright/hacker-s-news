"use client";

import { useEffect, useState, useRef, useCallback } from 'react';
import { Item } from '../src/types';

function formatTime(seconds: number): string {
  if (!seconds || isNaN(seconds)) return "00:00";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
}

function getRelativeTime(timestamp: number): string {
  if (!timestamp) return '';
  const now = Date.now();
  const diff = now - timestamp * 1000;

  if (diff < 60000) return 'Just now';
  const mins = Math.floor(diff / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return new Date(timestamp * 1000).toLocaleDateString();
}

export default function Home() {
  const [items, setItems] = useState<Item[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isDragging, setIsDragging] = useState(false);

  // Pagination State
  const [page, setPage] = useState(1);
  const [hasMore, setHasMore] = useState(true);
  const [isLoading, setIsLoading] = useState(false);

  // Persistence State
  const [playedIds, setPlayedIds] = useState<Set<string>>(new Set());
  const [initialized, setInitialized] = useState(false);
  const [resumeTime, setResumeTime] = useState<number | null>(null);
  const [showPlaylist, setShowPlaylist] = useState(false);

  // Load persistence
  useEffect(() => {
    try {
      const storedPlayed = localStorage.getItem('freshloop_played_ids');
      if (storedPlayed) {
        setPlayedIds(new Set(JSON.parse(storedPlayed)));
      }

      const storedResumeId = localStorage.getItem('freshloop_resume_id');
      const storedResumeTime = localStorage.getItem('freshloop_resume_time');
      if (storedResumeId) setCurrentId(storedResumeId);
      if (storedResumeTime) {
        const t = parseFloat(storedResumeTime);
        setProgress(t);
        setResumeTime(t); // Signal to seek
      }
    } catch (e) {
      console.error("Failed to load persistence", e);
    }
    setInitialized(true);
  }, []);

  // Save persistence (Played IDs)
  useEffect(() => {
    if (!initialized) return;
    try {
      localStorage.setItem('freshloop_played_ids', JSON.stringify(Array.from(playedIds)));
    } catch (e) {
      console.error("Failed to save played ids", e);
    }
  }, [playedIds, initialized]);

  // Save persistence (Resume State)
  useEffect(() => {
    if (!initialized || !currentId) return;
    localStorage.setItem('freshloop_resume_id', currentId);
    // We save progress frequently or on meaningful change?
    // Let's save it in specific events or throttled.
    // For simplicity here, rely on progress state update (every ~200ms).
    localStorage.setItem('freshloop_resume_time', progress.toString());
  }, [currentId, progress, initialized]);

  // Sync Audio Src (Declarative)
  useEffect(() => {
    if (!currentId || items.length === 0) return;
    const item = items.find(i => i.id === currentId);
    if (item?.audio_url && audioRef.current) {
      // Only update if changed to avoid reloading
      const currentSrc = audioRef.current.src;
      // Check if src matches (ignoring base if needed, but audio_url is absolute usually)
      if (currentSrc !== item.audio_url && !currentSrc.endsWith(item.audio_url)) {
        audioRef.current.src = item.audio_url;
        // If we have a resume time for THIS item (could match resumption), trigger seek.
        // But we only have global "last resume time".
        // If we just loaded the page (resumeTime is set), seek.
        if (resumeTime !== null) {
          audioRef.current.currentTime = resumeTime;
          setResumeTime(null); // Clear signal
        }
      }
    }
  }, [currentId, items]); // removed resumeTime dependency to avoid seek loops, handle inside

  // Audio ref
  const audioRef = useRef<HTMLAudioElement>(null);
  const observerTarget = useRef<HTMLDivElement>(null);

  // Weather State
  const [weather, setWeather] = useState<{ temp: number, code: number } | null>(null);

  useEffect(() => {
    if ("geolocation" in navigator) {
      navigator.geolocation.getCurrentPosition(async (position) => {
        try {
          const { latitude, longitude } = position.coords;
          const res = await fetch(`https://api.open-meteo.com/v1/forecast?latitude=${latitude}&longitude=${longitude}&current_weather=true`);
          const data = await res.json();
          if (data.current_weather) {
            setWeather({
              temp: Math.round(data.current_weather.temperature),
              code: data.current_weather.weathercode
            });
          }
        } catch (e) {
          console.error("Weather fetch failed", e);
        }
      });
    }
  }, []);

  const getWeatherIcon = (code: number) => {
    if (code === 0) return 'wb_sunny';
    if (code <= 3) return 'partly_cloudy_day';
    if (code <= 48) return 'foggy';
    if (code <= 67) return 'rainy';
    if (code <= 77) return 'ac_unit';
    if (code <= 82) return 'rainy';
    if (code <= 86) return 'ac_unit';
    return 'thunderstorm';
  };

  const fetchItems = useCallback(async (pageNum: number, isRefresh = false) => {
    setIsLoading(true);
    try {
      const res = await fetch(`/api/items?page=${pageNum}&limit=20`);
      const data = await res.json();

      if (isRefresh) {
        setItems(data);
        setPage(1);
      } else {
        // filter out potentially duplicate ids just in case
        setItems(prev => {
          const newItems = data.filter((d: Item) => !prev.some(p => p.id === d.id));
          return [...prev, ...newItems];
        });
      }

      setHasMore(data.length === 20);
    } catch (err) {
      console.error('Failed to fetch items:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchItems(1, true);
  }, []);

  useEffect(() => {
    const observer = new IntersectionObserver(
      entries => {
        if (entries[0].isIntersecting && hasMore && !isLoading) {
          const nextPage = page + 1;
          setPage(nextPage);
          fetchItems(nextPage, false);
        }
      },
      { threshold: 0.1 }
    );

    if (observerTarget.current) {
      observer.observe(observerTarget.current);
    }

    return () => observer.disconnect();
  }, [hasMore, isLoading, page, fetchItems]);

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
    // Mark as played
    if (!playedIds.has(id)) {
      const newPlayed = new Set(playedIds);
      newPlayed.add(id);
      setPlayedIds(newPlayed);
    }

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
    if (currentIndex === -1) return;

    // Chronological Next = Newer = Lower Index (since list is New->Old)
    // Find first unplayed item moving towards 0
    for (let i = currentIndex - 1; i >= 0; i--) {
      const item = items[i];
      if (item.audio_url && !playedIds.has(item.id)) {
        playItem(item.id, item.audio_url);
        return;
      }
    }
  }, [currentId, items, playedIds]);

  const playPrev = useCallback(() => {
    if (!currentId) return;
    const currentIndex = items.findIndex(i => i.id === currentId);
    if (currentIndex === -1) return;

    // Chronological Prev = Older = Higher Index
    for (let i = currentIndex + 1; i < items.length; i++) {
      const item = items[i];
      if (item.audio_url) {
        playItem(item.id, item.audio_url);
        return;
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
  const unreadCount = items.filter(i => !playedIds.has(i.id)).length;
  const lastUpdated = items.length > 0 && items[0].publish_time
    ? `Updated ${getRelativeTime(items[0].publish_time)}`
    : (items.length > 0 ? 'Updated recently' : 'No content');

  return (
    <div className="relative flex h-full min-h-screen w-full flex-col overflow-x-hidden max-w-md mx-auto shadow-2xl pb-32 bg-background-dark text-white font-display">
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
      <header className="sticky top-0 z-20 bg-background-dark/95 backdrop-blur-md px-4 pt-12 pb-4 border-b border-white/5">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[28px] font-bold leading-none tracking-tight text-white">FreshLoop</h1>
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
              <div className="flex items-center gap-3">
                {weather && <span className="text-white text-2xl font-bold tracking-tighter">{weather.temp}°</span>}
                <div className="h-10 w-10 rounded-full bg-primary/20 flex items-center justify-center text-primary">
                  <span className="material-symbols-outlined">{weather ? getWeatherIcon(weather.code) : 'wb_sunny'}</span>
                </div>
              </div>
            </div>
            <div className="space-y-4">
              <div className="flex items-center gap-4">
                <span className="text-5xl font-bold text-primary font-display tabular-nums leading-none">{unreadCount}</span>
                <div className="flex flex-col">
                  <span className="text-lg font-bold text-white leading-tight">Fresh stories</span>
                  <span className="text-sm font-medium text-white/70">Tailored for you</span>
                </div>
                <button
                  onClick={() => fetchItems(1, true)}
                  disabled={isLoading}
                  className="h-10 w-10 ml-2 rounded-full bg-white/10 hover:bg-white/20 flex items-center justify-center text-white transition-colors active:scale-95 disabled:opacity-50"
                  title="Refresh Feed"
                >
                  <span className={`material-symbols-outlined ${isLoading ? 'animate-spin' : ''}`}>refresh</span>
                </button>
              </div>
              <div className="flex items-center justify-between text-sm text-[#93c8a8]">
                <span className="flex items-center gap-1.5">
                  <span className="material-symbols-outlined text-[16px]">schedule</span>
                  {lastUpdated}
                </span>
              </div>
            </div>
          </div>
        </section>

        {/* Collections Section (Items List) */}
        <section className="flex flex-col gap-4">
          <div className="flex items-center justify-between px-1">
            <h3 className="text-xl font-bold text-white">Your Feed</h3>
          </div>

          <div className="flex flex-col gap-3">
            {items.map((item, index) => {
              const isActive = currentId === item.id;
              return (
                <div
                  key={item.id}
                  onClick={() => item.audio_url && playItem(item.id, item.audio_url)}
                  className={`
                      group flex items-center gap-4 bg-surface-dark p-4 rounded-2xl ring-1 shadow-sm hover:shadow-md transition-all cursor-pointer active:scale-[0.99]
                      ${isActive ? 'ring-primary' : 'ring-white/5 hover:ring-primary/50'}
                    `}
                >
                  <div className="relative shrink-0">
                    <div className={`flex items-center justify-center rounded-xl size-14 shadow-inner ${isActive ? 'bg-primary text-black' : 'bg-[#244732] text-white'}`}>
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
                    <h4 className={`text-base font-bold leading-tight truncate ${isActive ? 'text-primary' : 'text-white'}`}>
                      {item.title}
                    </h4>
                    <p className="text-[#93c8a8] text-sm mt-1 line-clamp-1">
                      {item.summary || "Audio briefing available"}
                    </p>
                    <div className="flex items-center gap-3 mt-2.5">
                      <div className="flex items-center gap-1.5 text-xs font-medium text-[#93c8a8] bg-black/20 px-2 py-0.5 rounded-md">
                        <span className="material-symbols-outlined text-[14px]">schedule</span>
                        {item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : 'Now'}
                      </div>
                      {item.duration_sec ? (
                        <div className="flex items-center gap-1.5 text-xs font-medium text-[#93c8a8] bg-black/20 px-2 py-0.5 rounded-md">
                          <span className="material-symbols-outlined text-[14px]">graphic_eq</span>
                          <span>{formatTime(item.duration_sec)}</span>
                        </div>
                      ) : null}
                    </div>
                  </div>
                  <div className="shrink-0">
                    <button className={`flex items-center justify-center size-10 rounded-full transition-colors ${isActive && isPlaying ? 'bg-primary text-black' : 'bg-black/20 text-white group-hover:bg-primary group-hover:text-black'}`}>
                      <span className="material-symbols-outlined filled text-[24px]">
                        {isActive && isPlaying ? 'pause' : 'play_arrow'}
                      </span>
                    </button>
                  </div>
                </div>
              );
            })}

            {items.length === 0 && !isLoading && (
              <div className="text-center py-20 text-[#93c8a8]">
                No stories found.
              </div>
            )}

            <div ref={observerTarget} className="h-4 w-full flex-shrink-0" />

            {isLoading && (
              <div className="flex justify-center py-4">
                <div className="size-6 border-2 border-white/20 border-t-primary rounded-full animate-spin"></div>
              </div>
            )}

            {!hasMore && items.length > 0 && (
              <div className="text-center py-6 text-sm text-white/30">
                You've reached the end
              </div>
            )}
          </div>
        </section>

      </main>

      {/* Persistent Player Bar (Floating) */}
      {currentItem && (
        <div className="fixed bottom-6 left-0 right-0 px-4 z-40 max-w-md mx-auto">
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
                <p className="text-white text-sm font-bold truncate">{currentItem.title}</p>
              </div>
              <p className="text-[#93c8a8] text-xs truncate">
                {formatTime(duration - progress)} remaining
              </p>
            </div>

            <div className="flex items-center gap-1">
              <button onClick={() => setShowPlaylist(true)} className="text-white/60 hover:text-white p-2 rounded-full transition-colors">
                <span className="material-symbols-outlined text-[26px]">queue_music</span>
              </button>
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

      {/* Playlist Overlay */}
      {showPlaylist && (
        <div className="fixed inset-0 z-50 flex flex-col items-center justify-end sm:justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200">
          <div className="w-full max-w-md bg-surface-dark rounded-3xl shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col animate-in slide-in-from-bottom-10 duration-200">
            <div className="flex items-center justify-between p-4 border-b border-white/5 shrink-0">
              <h3 className="text-lg font-bold text-white pl-2">Play Queue</h3>
              <button
                onClick={() => setShowPlaylist(false)}
                className="p-2 rounded-full hover:bg-white/10 text-white/60 hover:text-white transition-colors"
              >
                <span className="material-symbols-outlined">close</span>
              </button>
            </div>

            <div className="overflow-y-auto p-2 space-y-1">
              {[...items].reverse().map((item) => {
                const isActive = currentId === item.id;
                const isPlayed = playedIds.has(item.id);
                return (
                  <div
                    key={item.id}
                    onClick={() => {
                      playItem(item.id, item.audio_url || '');
                    }}
                    className={`flex items-center gap-3 p-3 rounded-xl cursor-pointer transition-colors ${isActive ? 'bg-primary/10' : 'hover:bg-white/5'}`}
                  >
                    <div className={`flex items-center justify-center size-10 rounded-lg shrink-0 ${isActive ? 'bg-primary text-black' : 'bg-white/5 text-white/40'}`}>
                      <span className="material-symbols-outlined text-xl">
                        {isActive && isPlaying ? 'graphic_eq' : (isPlayed ? 'check' : 'music_note')}
                      </span>
                    </div>
                    <div className="min-w-0 grow">
                      <h4 className={`text-sm font-bold truncate ${isActive ? 'text-primary' : (isPlayed ? 'text-white/40' : 'text-white')}`}>
                        {item.title}
                      </h4>
                      <p className="text-xs text-white/30 truncate">
                        {item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : 'Unknown time'}
                      </p>
                    </div>
                  </div>
                );
              })}
              {items.length === 0 && (
                <div className="py-8 text-center text-white/30 text-sm">queue is empty</div>
              )}
            </div>
          </div>
        </div>
      )}

    </div>
  );
}

