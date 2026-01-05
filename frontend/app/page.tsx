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
  const [showTranscript, setShowTranscript] = useState(false);

  // Sources Modal State
  const [showSources, setShowSources] = useState(false);
  const [sources, setSources] = useState<Array<{ url: string, title: string, summary: string }>>([]);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [sourcesItemId, setSourcesItemId] = useState<string | null>(null);

  // Playlist and Transcript UI State
  const [showPlayed, setShowPlayed] = useState(false);
  const [transcriptItemId, setTranscriptItemId] = useState<string | null>(null);
  const [queueIds, setQueueIds] = useState<string[]>([]);
  // Debug State
  const [showDebug, setShowDebug] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);

  // Capture Logs
  useEffect(() => {
    const originalLog = console.log;
    const originalWarn = console.warn;
    const originalError = console.error;

    const addLog = (type: string, args: any[]) => {
      const msg = args.map(arg =>
        typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)
      ).join(' ');
      const time = new Date().toLocaleTimeString();
      setLogs(prev => [...prev.slice(-99), `[${time}] [${type}] ${msg}`]);
    };

    console.log = (...args) => {
      originalLog(...args);
      addLog('LOG', args);
    };
    console.warn = (...args) => {
      originalWarn(...args);
      addLog('WARN', args);
    };
    console.error = (...args) => {
      originalError(...args);
      addLog('ERR', args);
    };

    return () => {
      console.log = originalLog;
      console.warn = originalWarn;
      console.error = originalError;
    };
  }, []);

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
    // Don't save if we are in the middle of a resume-seek operation (to avoid overwriting with 0)
    if (resumeTime !== null) return;

    localStorage.setItem('freshloop_resume_id', currentId);
    // We save progress frequently or on meaningful change?
    // Let's save it in specific events or throttled.
    // For simplicity here, rely on progress state update (every ~200ms).
    localStorage.setItem('freshloop_resume_time', progress.toString());
  }, [currentId, progress, initialized, resumeTime]);



  // Audio ref
  const audioRef = useRef<HTMLAudioElement>(null);
  const observerTarget = useRef<HTMLDivElement>(null);

  // Weather State
  const [weather, setWeather] = useState<{ temp: number, code: number } | null>(null);
  const [greeting, setGreeting] = useState('Good Morning');

  useEffect(() => {
    const updateGreeting = () => {
      const hour = new Date().getHours();
      if (hour < 5) setGreeting("Good Late Night");
      else if (hour < 12) setGreeting("Good Morning");
      else if (hour < 17) setGreeting("Good Afternoon");
      else if (hour < 21) setGreeting("Good Evening");
      else setGreeting("Good Night");
    };

    updateGreeting(); // Initial call
    const intervalId = setInterval(updateGreeting, 60000); // Update every minute

    return () => clearInterval(intervalId); // Cleanup on unmount
  }, []);

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
        // Items from API are New -> Old
        // User wants Old -> New order for general flow
        // Only add items that are NOT played
        const initialQueue = data.map((i: Item) => i.id).reverse()
          .filter((id: string) => !playedIds.has(id));

        // If there's a current playing item, ensure it's in the queue
        if (currentId && !playedIds.has(currentId) && !initialQueue.includes(currentId)) {
          initialQueue.unshift(currentId);
        }

        setQueueIds(initialQueue);

        // Preserve currently playing item to avoid audio interruption
        setItems(prev => {
          const currentPlayingItem = prev.find(i => i.id === currentId);
          const newItems = data.filter((d: Item) => d.id !== currentId);
          if (currentPlayingItem) {
            const currentInNew = data.find((d: Item) => d.id === currentId);
            if (currentInNew) return data;
            return [currentPlayingItem, ...newItems];
          }
          return data;
        });
        setPage(1);
      } else {
        // Append new items to the END of the queue (since they are newer)
        const newIds = data.map((i: Item) => i.id).reverse()
          .filter((id: string) => !playedIds.has(id) && !queueIds.includes(id));

        setQueueIds(prev => [...prev, ...newIds]);

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
  }, [currentId, playedIds, queueIds]);

  useEffect(() => {
    if (initialized) {
      fetchItems(1, true);
    }
  }, [initialized]);

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

  // Fetch sources for an item
  const fetchSources = async (itemId: string) => {
    setSourcesLoading(true);
    setSourcesItemId(itemId);
    setShowSources(true);
    try {
      const res = await fetch(`/api/items/${itemId}/sources`);
      if (res.ok) {
        const data = await res.json();
        setSources(data.map((s: any) => ({
          url: s.source_url,
          title: s.source_title || 'Untitled',
          summary: s.source_summary || ''
        })));
      } else {
        setSources([]);
      }
    } catch (e) {
      console.error('Failed to fetch sources', e);
      setSources([]);
    } finally {
      setSourcesLoading(false);
    }
  };

  // Audio Event Handlers
  // Sync audio element with state
  // Master Audio Effect: Sync Src, Resume, and Play State
  useEffect(() => {
    if (!audioRef.current || !initialized) return;
    if (!currentId || items.length === 0) return;

    const item = items.find(i => i.id === currentId);
    if (!item || !item.audio_url) return;

    // 1. Sync Source
    const currentSrc = audioRef.current.src;
    // Use absolute URL comparison to be safe
    const targetSrc = new URL(item.audio_url, window.location.href).href;
    const srcChanged = currentSrc !== targetSrc;

    if (srcChanged) {
      audioRef.current.src = item.audio_url;
      // Note: resumeTime seek will be handled by handleLoadedMetadata
      // But we can also set it here if readyState > 0? Best to rely on metadata event.
      audioRef.current.load();
    }

    // 2. Sync Play/Pause State
    if (isPlaying) {
      audioRef.current.play().catch(e => {
        console.warn("Playback failed or interrupted", e);
      });
    } else {
      audioRef.current.pause();
    }
  }, [currentId, isPlaying, initialized, items]);

  const togglePlay = () => {
    setIsPlaying(!isPlaying);
  };

  // Helper to mark item as read
  const markAsPlayed = useCallback((id: string) => {
    setPlayedIds(prev => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
    setQueueIds(prev => prev.filter(qid => qid !== id));

    // If we marked the CURRENT playing item as played manually, play the next one? 
    // Usually user does this for items they don't want to hear.
    if (currentId === id) {
      // if we are playing it, and user marks as read, maybe skip next?
      // Let's defer to playNext logic.
    }
  }, [currentId]);

  const playItem = useCallback((id: string, url: string) => {
    if (currentId === id) {
      setIsPlaying(!isPlaying);
    } else {
      // If it was played, re-add to queue at start
      if (playedIds.has(id)) {
        setPlayedIds(prev => {
          const next = new Set(prev);
          next.delete(id);
          return next;
        });
        setQueueIds(prev => [id, ...prev.filter(qid => qid !== id)]);
      } else if (!queueIds.includes(id)) {
        // If not in queue (somehow?), add it
        setQueueIds(prev => [id, ...prev]);
      }

      setCurrentId(id);
      setIsPlaying(true);
    }
  }, [currentId, isPlaying, playedIds, queueIds]);

  const playNext = useCallback(() => {
    console.log("[AutoPlay] playNext triggered", { currentId, queueIds });
    if (!currentId) return;

    // 1. Mark FINISHED item as played
    setPlayedIds(prev => {
      const next = new Set(prev);
      next.add(currentId);
      return next;
    });

    // 2. Remove FINISHED item from queue
    setQueueIds(prev => {
      const nextQueue = prev.filter(id => id !== currentId);
      return nextQueue;
    });

    // 3. Find next item ID to play from current queue state
    const nextQueue = queueIds.filter(id => id !== currentId);

    let nextId = null;
    if (nextQueue.length > 0) {
      nextId = nextQueue[0];
    }

    if (nextId) {
      console.log("[AutoPlay] Playing next:", nextId);
      setCurrentId(nextId);
      setIsPlaying(true);
    } else {
      console.log("[AutoPlay] Queue empty, stopping.");
      setIsPlaying(false);
    }
  }, [currentId, queueIds]);

  const playPrev = useCallback(() => {
    if (!currentId) return;
    const currentIndex = queueIds.indexOf(currentId);
    if (currentIndex > 0) {
      setCurrentId(queueIds[currentIndex - 1]);
      setIsPlaying(true);
    }
  }, [currentId, queueIds]);

  const handleTimeUpdate = () => {
    // Block updates if we are waiting to resume seeking, to prevent overwriting saved progress with 0
    if (resumeTime !== null) return;

    if (audioRef.current && !isDragging) {
      setProgress(audioRef.current.currentTime);
    }
  };

  const handleLoadedMetadata = () => {
    if (audioRef.current) {
      setDuration(audioRef.current.duration);
      if (resumeTime !== null) {
        audioRef.current.currentTime = resumeTime;
        setResumeTime(null);
      }
    }
  };

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const time = parseFloat(e.target.value);
    if (audioRef.current) {
      audioRef.current.currentTime = time;
      setProgress(time);
    }
  };

  const skipTime = (seconds: number) => {
    if (audioRef.current) {
      const newTime = audioRef.current.currentTime + seconds;
      audioRef.current.currentTime = Math.max(0, Math.min(newTime, duration));
      setProgress(audioRef.current.currentTime);
    }
  };

  // Debug Trigger Logic
  const debugClicks = useRef(0);
  const lastDebugClick = useRef(0);
  const handleDebugTrigger = () => {
    const now = Date.now();
    // Reset if too slow (more than 500ms between clicks)
    if (now - lastDebugClick.current > 500) {
      debugClicks.current = 0;
    }

    debugClicks.current += 1;
    lastDebugClick.current = now;

    if (debugClicks.current >= 5) {
      setShowDebug(prev => !prev);
      debugClicks.current = 0;
      // Visual feedback?
      if (navigator.vibrate) navigator.vibrate(50);
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
        onEnded={() => {
          console.log("[Audio] onEnded fired");
          playNext();
        }}
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        className="hidden"
        autoPlay
        playsInline
      />

      {/* Header */}
      <header className="sticky top-0 z-20 bg-background-dark/95 backdrop-blur-md px-4 pt-12 pb-4 border-b border-white/5">
        <div className="flex items-center justify-between">
          <div>
            <div className="flex items-center gap-2" onClick={handleDebugTrigger}>
              <h1 className="text-[28px] font-bold leading-none tracking-tight text-white">FreshLoop</h1>
              {showDebug && <div className="w-2 h-2 rounded-full bg-red-500 animate-pulse" />}
            </div>
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
                <h2 className="text-3xl font-bold text-white tracking-tight leading-none">{greeting}</h2>
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
            {items.filter(i => !playedIds.has(i.id)).map((item, index) => {
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
                  <div className="flex flex-col items-center gap-2 shrink-0">
                    <button className={`flex items-center justify-center size-10 rounded-full transition-colors ${isActive && isPlaying ? 'bg-primary text-black' : 'bg-black/20 text-white group-hover:bg-primary group-hover:text-black'}`}>
                      <span className="material-symbols-outlined filled text-[24px]">
                        {isActive && isPlaying ? 'pause' : 'play_arrow'}
                      </span>
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setTranscriptItemId(item.id);
                        setShowTranscript(true);
                      }}
                      className="flex items-center justify-center size-8 rounded-full bg-black/20 text-white/60 hover:text-white hover:bg-black/30 transition-colors"
                      title="查看文稿"
                    >
                      <span className="material-symbols-outlined text-[18px]">article</span>
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

            <div className="flex items-center gap-0.5">
              <button
                onClick={() => {
                  setTranscriptItemId(currentId);
                  setShowTranscript(true);
                }}
                className="text-white/60 hover:text-white p-1.5 rounded-full transition-colors"
                title="Read Transcript"
              >
                <span className="material-symbols-outlined text-[24px]">article</span>
              </button>
              <button onClick={() => setShowPlaylist(true)} className="text-white/60 hover:text-white p-1.5 rounded-full transition-colors">
                <span className="material-symbols-outlined text-[24px]">queue_music</span>
              </button>
              <button onClick={togglePlay} className="text-white hover:text-primary p-1 rounded-full transition-colors">
                <span className="material-symbols-outlined text-[32px] filled">
                  {isPlaying ? 'pause_circle' : 'play_circle'}
                </span>
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Playlist Overlay */}
      {showPlaylist && (
        <div className="fixed inset-0 z-50 flex flex-col items-center justify-end sm:justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200" onClick={() => setShowPlaylist(false)}>
          <div className="w-full max-w-md bg-surface-dark rounded-3xl shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col animate-in slide-in-from-bottom-10 duration-200 overscroll-contain" onClick={(e) => e.stopPropagation()}>
            <div className="flex flex-col p-3 border-b border-white/5 shrink-0 bg-surface-dark/95 backdrop-blur z-10 rounded-t-3xl gap-1">
              <div className="flex items-center justify-between">
                <h3 className="text-lg font-bold text-white pl-1">播放列表</h3>
                <button
                  onClick={() => setShowPlaylist(false)}
                  className="p-1.5 rounded-full hover:bg-white/10 text-white/60 hover:text-white transition-colors"
                >
                  <span className="material-symbols-outlined">close</span>
                </button>
              </div>

              <div className="flex flex-col gap-2 pb-1">
                {/* Progress Bar */}
                <div className="flex flex-col gap-1 px-2">
                  <input
                    type="range"
                    min="0"
                    max={duration || 100}
                    value={progress}
                    onChange={handleSeek}
                    className="w-full h-1 bg-white/10 rounded-lg appearance-none cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
                  />
                  <div className="flex justify-between text-[10px] text-white/40 font-mono">
                    <span>{formatTime(progress)}</span>
                    <span>{formatTime(duration)}</span>
                  </div>
                </div>

                <div className="flex items-center justify-center gap-4">
                  {/* Article / Transcript */}
                  <button
                    onClick={() => {
                      setTranscriptItemId(currentId);
                      setShowTranscript(true);
                    }}
                    className="text-white/40 hover:text-white transition-colors p-2"
                    title="View Transcript"
                  >
                    <span className="material-symbols-outlined text-[24px]">article</span>
                  </button>

                  {/* Rewind */}
                  <button onClick={() => skipTime(-15)} className="text-white/40 hover:text-white transition-colors p-2">
                    <span className="material-symbols-outlined text-[24px]">replay_10</span>
                  </button>

                  <button onClick={playPrev} className="text-white/60 hover:text-white transition-colors p-2">
                    <span className="material-symbols-outlined text-[32px]">skip_previous</span>
                  </button>
                  <button onClick={togglePlay} className="text-white hover:text-primary transition-colors p-1 rounded-full">
                    <span className="material-symbols-outlined text-[48px] filled">
                      {isPlaying ? 'pause_circle' : 'play_circle'}
                    </span>
                  </button>
                  <button onClick={playNext} className="text-white/60 hover:text-white transition-colors p-2">
                    <span className="material-symbols-outlined text-[32px]">skip_next</span>
                  </button>

                  {/* Fast Forward */}
                  <button onClick={() => skipTime(30)} className="text-white/40 hover:text-white transition-colors p-2">
                    <span className="material-symbols-outlined text-[24px]">forward_30</span>
                  </button>
                </div>
              </div>
            </div>

            <div className="overflow-y-auto p-2 space-y-4">
              {/* Queue (Unplayed + Currently Playing) */}
              {(() => {
                const queueItems = queueIds
                  .map(id => items.find(i => i.id === id))
                  .filter(i => i !== undefined) as Item[];

                return queueItems.length > 0 && (
                  <div>
                    <h4 className="text-xs font-bold text-[#93c8a8] uppercase tracking-wider px-2 py-2">
                      播放队列 ({queueItems.length})
                    </h4>
                    <div className="space-y-1">
                      {queueItems.map((item) => {
                        const isActive = currentId === item.id;
                        return (
                          <div
                            key={item.id}
                            onClick={() => playItem(item.id, item.audio_url || '')}
                            className={`flex items-center gap-3 p-3 rounded-xl cursor-pointer transition-colors ${isActive ? 'bg-primary/10' : 'hover:bg-white/5'}`}
                          >
                            <div className={`flex items-center justify-center size-10 rounded-lg shrink-0 ${isActive ? 'bg-primary text-black' : 'bg-white/5 text-white/40'}`}>
                              <span className="material-symbols-outlined text-xl">
                                {isActive && isPlaying ? 'graphic_eq' : 'music_note'}
                              </span>
                            </div>
                            <div className="min-w-0 grow">
                              <h4 className={`text-sm font-bold truncate ${isActive ? 'text-primary' : 'text-white'}`}>
                                {item.title}
                              </h4>
                              <div className="flex items-center gap-2 mt-0.5">
                                <div className="flex items-center gap-1 text-[10px] text-white/30 uppercase font-mono">
                                  <span className="material-symbols-outlined leading-3" style={{ fontSize: '10px' }}>schedule</span>
                                  <span className="leading-3">{item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}</span>
                                </div>
                                {item.duration_sec && (
                                  <div className="flex items-center gap-1 text-[10px] text-[#93c8a8] font-mono">
                                    <span className="material-symbols-outlined leading-3" style={{ fontSize: '10px' }}>timer</span>
                                    <span className="leading-3">{formatTime(item.duration_sec)}</span>
                                  </div>
                                )}
                              </div>
                            </div>

                            <button
                              onClick={(e) => {
                                e.stopPropagation();
                                markAsPlayed(item.id);
                              }}
                              className="p-2 rounded-full text-white/20 hover:text-white hover:bg-white/10 transition-all shrink-0"
                              title="Mark as Played"
                            >
                              <span className="material-symbols-outlined" style={{ fontSize: '20px' }}>check_circle</span>
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })()}

              {/* Played (History) - Collapsible */}
              {(() => {
                const playedItems = items
                  .filter(i => playedIds.has(i.id) && i.id !== currentId)
                  .sort((a, b) => (b.publish_time || 0) - (a.publish_time || 0));

                return playedItems.length > 0 && (
                  <div className="border-t border-white/5 mt-2 pt-2">
                    <button
                      onClick={() => setShowPlayed(!showPlayed)}
                      className="w-full flex items-center justify-between px-2 py-1.5 text-white/30 hover:text-white/50 transition-colors"
                    >
                      <span className="text-xs font-bold uppercase tracking-wider">
                        已播放 ({playedItems.length})
                      </span>
                      <span className="material-symbols-outlined text-sm">
                        {showPlayed ? 'expand_less' : 'expand_more'}
                      </span>
                    </button>
                    {showPlayed && (
                      <div className="space-y-1 mt-1">
                        {playedItems.slice(0, 20).map((item) => (
                          <div
                            key={item.id}
                            onClick={() => playItem(item.id, item.audio_url || '')}
                            className="flex items-center gap-3 p-3 rounded-xl cursor-pointer transition-colors hover:bg-white/5"
                          >
                            <span className="material-symbols-outlined text-white/20 text-lg">replay</span>
                            <div className="min-w-0 grow">
                              <h4 className="text-sm font-medium truncate text-white/40">
                                {item.title}
                              </h4>
                              <div className="flex items-center gap-2 mt-0.5">
                                <div className="flex items-center gap-1 text-[10px] text-white/20">
                                  <span className="material-symbols-outlined leading-3" style={{ fontSize: '10px' }}>schedule</span>
                                  <span className="leading-3">{item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}</span>
                                </div>
                                {item.duration_sec && (
                                  <div className="flex items-center gap-1 text-[10px] text-white/10 font-mono">
                                    <span className="material-symbols-outlined leading-3" style={{ fontSize: '10px' }}>timer</span>
                                    <span className="leading-3">{formatTime(item.duration_sec)}</span>
                                  </div>
                                )}
                              </div>
                            </div>
                          </div>
                        ))}
                        {playedItems.length > 20 && (
                          <div className="text-center py-1 text-xs text-white/20">
                            还有 {playedItems.length - 20} 条
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                );
              })()}

              {items.length === 0 && (
                <div className="py-8 text-center text-white/30 text-sm">暂无内容</div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Debug Console Overlay */}
      {showDebug && (
        <div className="fixed inset-x-0 bottom-0 z-[100] h-[50vh] bg-black/90 text-green-400 font-mono text-[10px] p-2 overflow-y-auto border-t border-white/20 pointer-events-auto">
          <div className="flex justify-between items-center bg-white/10 p-1 mb-2 rounded">
            <span className="font-bold text-white">Debug Console ({logs.length})</span>
            <div className="flex gap-2">
              <button onClick={() => {
                const text = logs.join('\n');
                navigator.clipboard.writeText(text).then(() => alert('Logs copied!'));
              }} className="px-2 py-1 bg-blue-600 text-white rounded">Copy</button>
              <button onClick={() => setLogs([])} className="px-2 py-1 bg-white/20 text-white rounded">Clear</button>
              <button onClick={() => setShowDebug(false)} className="px-2 py-1 bg-red-600 text-white rounded">Close</button>
            </div>
          </div>
          <div className="whitespace-pre-wrap break-all">
            {logs.map((log, i) => (
              <div key={i} className="border-b border-white/5 py-0.5">{log}</div>
            ))}
            <div id="log-end" />
          </div>
        </div>
      )}

      {/* Transcript Overlay */}
      {showTranscript && (() => {
        const transcriptItem = transcriptItemId
          ? items.find(i => i.id === transcriptItemId) || currentItem
          : currentItem;
        return transcriptItem && (
          <div className="fixed inset-0 z-50 flex flex-col items-center justify-end sm:justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200">
            <div className="w-full max-w-2xl bg-surface-dark rounded-3xl shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col animate-in slide-in-from-bottom-10 duration-200 overscroll-contain">
              <div className="flex items-center justify-between p-6 border-b border-white/5 shrink-0 bg-surface-dark/50 z-10 rounded-t-3xl">
                <div className="pr-4">
                  <p className="text-[#93c8a8] text-xs font-bold uppercase tracking-wider mb-2">文稿</p>
                  <h3 className="text-xl font-bold text-white leading-tight">{transcriptItem.title}</h3>
                </div>
                <button
                  onClick={() => setShowTranscript(false)}
                  className="p-2 rounded-full hover:bg-white/10 text-white/60 hover:text-white transition-colors shrink-0"
                >
                  <span className="material-symbols-outlined">close</span>
                </button>
              </div>

              <div className="overflow-y-auto p-6 text-white/90">
                {transcriptItem.summary ? (
                  <div className="prose prose-invert prose-lg max-w-none">
                    <p className="whitespace-pre-wrap font-serif leading-relaxed text-[1.1rem]">
                      {transcriptItem.summary}
                    </p>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center py-20 text-white/30 text-center">
                    <span className="material-symbols-outlined text-4xl mb-4 opacity-50">description</span>
                    <p>暂无文稿内容</p>
                  </div>
                )}
              </div>
              <div className="p-4 border-t border-white/5 shrink-0 bg-surface-dark/50 rounded-b-3xl">
                <button
                  onClick={() => {
                    setShowTranscript(false);
                    fetchSources(transcriptItem.id);
                  }}
                  className="w-full flex items-center justify-center gap-2 text-primary hover:text-primary/80 transition-colors text-sm font-bold py-2 bg-primary/10 hover:bg-primary/20 rounded-xl"
                >
                  <span className="material-symbols-outlined text-[18px]">article</span>
                  查看原文来源
                </button>
              </div>
            </div>
          </div>
        );
      })()}

      {/* Sources Modal */}
      {showSources && (
        <div className="fixed inset-0 z-50 flex flex-col items-center justify-end sm:justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200">
          <div className="w-full max-w-2xl bg-surface-dark rounded-3xl shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col animate-in slide-in-from-bottom-10 duration-200">
            <div className="flex items-center justify-between p-6 border-b border-white/5 shrink-0 bg-surface-dark/50 z-10 rounded-t-3xl">
              <div className="pr-4">
                <p className="text-[#93c8a8] text-xs font-bold uppercase tracking-wider mb-2">原始来源</p>
                <h3 className="text-xl font-bold text-white leading-tight">
                  {sources.length} 篇参考文章
                </h3>
              </div>
              <button
                onClick={() => setShowSources(false)}
                className="p-2 rounded-full hover:bg-white/10 text-white/60 hover:text-white transition-colors shrink-0"
              >
                <span className="material-symbols-outlined">close</span>
              </button>
            </div>

            <div className="overflow-y-auto p-4">
              {sourcesLoading ? (
                <div className="flex justify-center py-12">
                  <div className="size-8 border-2 border-white/20 border-t-primary rounded-full animate-spin"></div>
                </div>
              ) : sources.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-16 text-white/30 text-center">
                  <span className="material-symbols-outlined text-4xl mb-4 opacity-50">article</span>
                  <p>暂无原始来源信息</p>
                </div>
              ) : (
                <div className="flex flex-col gap-3">
                  {sources.map((source, idx) => (
                    <div key={idx} className="bg-black/20 rounded-xl p-4 hover:bg-black/30 transition-colors">
                      <h4 className="text-white font-bold text-sm mb-2 line-clamp-2">{source.title}</h4>
                      <p className="text-white/60 text-xs mb-3 line-clamp-3">{source.summary}</p>
                      <a
                        href={source.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-flex items-center gap-1.5 text-primary hover:text-primary/80 text-xs font-bold transition-colors"
                      >
                        <span className="material-symbols-outlined text-[14px]">open_in_new</span>
                        查看原文
                      </a>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

    </div>
  );
}

