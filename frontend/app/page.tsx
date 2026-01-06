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

// Animated Equalizer Component for Playing State
const AnimatedEqualizer = ({ size = 'md', className = '' }: { size?: 'sm' | 'md' | 'lg'; className?: string }) => {
  const sizeConfig = {
    sm: { container: 'h-4', bar: 'w-0.5' },
    md: { container: 'h-5', bar: 'w-1' },
    lg: { container: 'h-6', bar: 'w-1.5' },
  };
  const cfg = sizeConfig[size];
  return (
    <div className={`flex items-end justify-center gap-0.5 ${cfg.container} animate-soundwave ${className}`}>
      <span className={`${cfg.bar} bg-current rounded-full origin-bottom`} style={{ height: '60%' }} />
      <span className={`${cfg.bar} bg-current rounded-full origin-bottom`} style={{ height: '100%' }} />
      <span className={`${cfg.bar} bg-current rounded-full origin-bottom`} style={{ height: '40%' }} />
      <span className={`${cfg.bar} bg-current rounded-full origin-bottom`} style={{ height: '80%' }} />
    </div>
  );
};

import { LoginModal } from '../components/LoginModal';

export default function Home() {
  // Auth State
  const [user, setUser] = useState<{ id: string; username: string } | null>(null);
  const [showLogin, setShowLogin] = useState(false);
  const [items, setItems] = useState<Item[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [isBuffering, setIsBuffering] = useState(false);
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
  const [isPlayerExpanded, setIsPlayerExpanded] = useState(false);
  const [showTranscript, setShowTranscript] = useState(false);

  // Sources Modal State
  const [showSources, setShowSources] = useState(false);
  const [sources, setSources] = useState<Array<{ url: string, title: string, summary: string }>>([]);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [sourcesItemId, setSourcesItemId] = useState<string | null>(null);

  // Playlist and Transcript UI State
  const [playedExpanded, setPlayedExpanded] = useState(false); // Played list collapsed by default
  const [panelView, setPanelView] = useState<'transcript' | 'playlist'>('transcript'); // Default to transcript
  const [transcriptItemId, setTranscriptItemId] = useState<string | null>(null);
  const [queueIds, setQueueIds] = useState<string[]>([]);
  // Debug State
  const [showDebug, setShowDebug] = useState(false);
  const [debugMinimized, setDebugMinimized] = useState(false);
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

  // Lock Body Scroll when Modal is Open
  useEffect(() => {
    if (isPlayerExpanded || showTranscript || showSources) {
      document.body.style.overflow = 'hidden';
    } else {
      document.body.style.overflow = '';
    }
    return () => {
      document.body.style.overflow = '';
    };
  }, [isPlayerExpanded, showTranscript, showSources]);

  // Load persistence (History from API + Resume from Local)
  useEffect(() => {
    // 1. Fetch History from Backend
    // 1. Fetch History from Backend
    const headers: HeadersInit = {};
    if (user) {
      headers['x-user-id'] = user.id;
    }

    fetch('/api/history', { headers })
      .then(res => res.json())
      .then((data: { item_id: string }[]) => {
        const ids = new Set(data.map(i => i.item_id));
        setPlayedIds(ids);
      })
      .catch(e => console.error("Failed to fetch history", e));

    // 2. Load Resume State (Keep Local)
    try {
      const storedResumeId = localStorage.getItem('freshloop_resume_id');
      const storedResumeTime = localStorage.getItem('freshloop_resume_time');
      if (storedResumeId) setCurrentId(storedResumeId);
      if (storedResumeTime) {
        const t = parseFloat(storedResumeTime);
        setProgress(t);
        setResumeTime(t);
      }
    } catch (e) {
      console.error("Failed to load local persistence", e);
    }
    setInitialized(true);
  }, [user]);



  // Save persistence (Resume State ONLY)
  useEffect(() => {
    if (!initialized || !currentId) return;
    if (resumeTime !== null) return;

    localStorage.setItem('freshloop_resume_id', currentId);
    localStorage.setItem('freshloop_resume_time', progress.toString());
    localStorage.setItem('freshloop_resume_id', currentId);
    localStorage.setItem('freshloop_resume_time', progress.toString());
  }, [currentId, progress, initialized, resumeTime]);

  // Restore Auth
  useEffect(() => {
    const storedUser = localStorage.getItem('freshloop_user');
    if (storedUser) {
      try {
        setUser(JSON.parse(storedUser));
      } catch (e) {
        console.error("Failed to parse stored user", e);
      }
    }
  }, []);

  const handleLogin = (u: { id: string; username: string }) => {
    setUser(u);
    localStorage.setItem('freshloop_user', JSON.stringify(u));
  };

  const handleLogout = () => {
    setUser(null);
    localStorage.removeItem('freshloop_user');
    setPlayedIds(new Set()); // Clear history view
  };



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

  // Derived sorted lists
  const pendingItems = items
    .filter(i => !playedIds.has(i.id))
    .sort((a, b) => (user ? (a.publish_time || 0) - (b.publish_time || 0) : (b.publish_time || 0) - (a.publish_time || 0))); // Old -> New

  // Correction: User requested Old->New (Oldest first). 
  // If a.time < b.time => -1 (a comes first). Correct.
  // Wait, usually feeds are New -> Old. 
  // User wrote: "待播放页始终按照从旧到新...". 
  // So if I have items from 10:00 and 11:00. 10:00 should play first. 

  const playedItems = items
    .filter(i => playedIds.has(i.id))
    .sort((a, b) => (b.publish_time || 0) - (a.publish_time || 0)); // New -> Old (History)

  // Fetch Items (Raw Data)
  const fetchItems = useCallback(async (pageNum: number, isRefresh = false) => {
    setIsLoading(true);
    try {
      const res = await fetch(`/api/items?page=${pageNum}&limit=50`); // Fetch more to ensure we have a good buffer
      const data = await res.json();

      setItems(prev => {
        if (isRefresh) return data;
        // Merge and dedup
        const seen = new Set(prev.map(i => i.id));
        const newItems = data.filter((d: Item) => !seen.has(d.id));
        return [...prev, ...newItems];
      });

      setHasMore(data.length === 50);
    } catch (err) {
      console.error('Failed to fetch items:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  // Initial Load (Auto-fetch)
  useEffect(() => {
    fetchItems(1);
  }, [fetchItems]);

  const markAsPlayed = useCallback((id: string) => {
    // 1. Optimistic Update
    setPlayedIds(prev => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });

    // 2. Backend Sync
    const headers: HeadersInit = { 'Content-Type': 'application/json' };
    if (user) headers['x-user-id'] = user.id;

    fetch('/api/history', {
      method: 'POST',
      headers,
      body: JSON.stringify({ item_id: id })
    }).catch(e => console.error("Failed to sync history", e));
  }, [user]);

  const playItem = useCallback((id: string, url: string) => {
    if (currentId === id) {
      setIsPlaying(!isPlaying);
    } else {
      setCurrentId(id);
      setIsPlaying(true);
    }
  }, [currentId, isPlaying]);

  const checkForMore = useCallback(async () => {
    console.log("[AutoPlay] Checking server for more content...");
    setIsLoading(true);
    try {
      // Try fetching page 1 again to see if new stuff arrived
      const res = await fetch(`/api/items?page=1&limit=20`);
      const data = await res.json();

      let hasNewParams = false;
      setItems(prev => {
        const seen = new Set(prev.map(i => i.id));
        const newItems = data.filter((d: Item) => !seen.has(d.id));
        if (newItems.length > 0) hasNewParams = true;
        return [...prev, ...newItems];
      });

      return hasNewParams;
    } catch (e) {
      return false;
    } finally {
      setIsLoading(false);
    }
  }, []);

  const playNext = useCallback(async () => {
    console.log("[AutoPlay] playNext triggered for", currentId);

    // 1. Mark current as played (moves it to History list)
    if (currentId) {
      markAsPlayed(currentId);
    }

    // 2. Determine Next Item
    // Since 'pendingItems' updates immediately upon 'markAsPlayed' (React state), 
    // inside this callback we might still see the OLD derived state if we rely on closure 'pendingItems'.
    // However, we need to decide NEXT based on the logical list.
    // The 'pendingItems' variable in scope is from the *last render*. 
    // If currentId was in pendingItems, it's about to be removed.
    // So the next item is logically the one *after* currentId in the sorted pending list.
    // OR, if pendingItems is Old->New, and we just finished currentId (which should be at top),
    // then the next one is indeed pendingItems[1] (if current is 0) or simply pendingItems[0] of the NEXT render.

    // BETTER APPROACH: Find the candidate from the generic 'items' pool using the same sort logic,
    // excluding the one we just finished.

    // We can't rely on 'pendingItems' in this closure updating instantly.
    // Manually filter:
    const nextCandidates = items
      .filter(i => !playedIds.has(i.id) && i.id !== currentId) // Remove played + just finished
      .sort((a, b) => (a.publish_time || 0) - (b.publish_time || 0)); // Old->New

    if (nextCandidates.length > 0) {
      const nextId = nextCandidates[0].id;
      console.log("[AutoPlay] Next item found:", nextId);
      setCurrentId(nextId);
      setIsPlaying(true);
    } else {
      console.log("[AutoPlay] Local queue empty. Checking server...");
      // Attempt to fetch more
      const foundNew = await checkForMore();
      if (!foundNew) {
        console.log("[AutoPlay] No new content from server. Stop.");
        setIsPlaying(false);
        // Don't clear currentId so player stays visible (as 'finished' state)
      } else {
        // If found new, we need to trigger playNext again? 
        // Or let the user wait? 
        // Ideally we auto-play the new stuff.
        // We can't recurse easily here without fresh state.
        // Just set isPlaying(false) for now, or try to find it blindly?
        // Let's rely on the user or a simpler re-check.
        setIsPlaying(false);
      }
    }
  }, [currentId, items, playedIds, markAsPlayed, checkForMore]);

  const playPrev = useCallback(() => {
    // History logic: New -> Old. 
    // Prev implies "Go back to the one I just heard" -> Top of Played List?
    // Or "Previous" in the Pending List? 
    // Standard player: Prev = Start of track OR Previous Track.
    // In this flow, "Previous" likely means "The most recently played item".
    const historyCandidates = items
      .filter(i => playedIds.has(i.id))
      .sort((a, b) => (b.publish_time || 0) - (a.publish_time || 0)); // New -> Old

    if (historyCandidates.length > 0) {
      setCurrentId(historyCandidates[0].id);
      setIsPlaying(true);
    }
  }, [items, playedIds]);

  const togglePlay = () => {
    setIsPlaying(!isPlaying);
  };

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

  // Audio Control Logic
  useEffect(() => {
    if (!audioRef.current) return;

    if (currentId) {
      const item = items.find(i => i.id === currentId);
      if (item && item.audio_url) {
        // Only update source if it has changed to prevent unwanted reloading
        const currentSrc = audioRef.current.getAttribute('src');
        if (currentSrc !== item.audio_url) {
          audioRef.current.src = item.audio_url;
          // Reset progress only when changing tracks
          setProgress(0);
          // Resume time handling is done in onLoadedMetadata
          audioRef.current.play()
            .then(() => setIsPlaying(true))
            .catch(e => console.error("Play failed", e));
        }
      }
    } else {
      audioRef.current.pause();
      setIsPlaying(false);
    }
  }, [currentId, items]);

  useEffect(() => {
    if (!audioRef.current) return;
    if (isPlaying) {
      audioRef.current.play().catch(e => console.error("Resume failed", e));
    } else {
      audioRef.current.pause();
    }
  }, [isPlaying]);

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
        onWaiting={() => setIsBuffering(true)}
        onCanPlay={() => setIsBuffering(false)}
        onPlaying={() => setIsBuffering(false)}
        className="hidden"
        autoPlay
        playsInline
      />

      {/* Header */}
      <header className="sticky top-0 z-20 bg-background-dark/95 backdrop-blur-md px-4 pt-12 pb-4 border-b border-white/5">
        <div className="flex items-center justify-between">
          <div>
            <div className="flex items-center gap-3" onClick={handleDebugTrigger}>
              <div className="size-10 rounded-xl overflow-hidden shadow-lg ring-1 ring-white/10">
                <img src="/logo.png" alt="FreshLoop Logo" className="w-full h-full object-cover" />
              </div>
              <h1 className="text-[28px] font-bold leading-none tracking-tight text-white">FreshLoop</h1>
              {showDebug && <div className="w-2 h-2 rounded-full bg-red-500 animate-pulse" />}
            </div>
            <p className="text-xs text-slate-500 dark:text-[#93c8a8] mt-1 font-medium tracking-wide uppercase">Audio Briefing • Zen Mode</p>
          </div>
          <button
            onClick={() => user ? (confirm('Logout?') && handleLogout()) : setShowLogin(true)}
            className="flex items-center gap-2 bg-white/5 hover:bg-white/10 px-3 py-1.5 rounded-full transition-colors"
          >
            <div className={`size-6 rounded-full flex items-center justify-center ${user ? 'bg-primary text-black' : 'bg-primary text-black'}`}>
              <span className="material-symbols-outlined text-[16px]">person</span>
            </div>
            {user && <span className="text-xs font-medium text-white/80">{user.username}</span>}
          </button>
        </div>
      </header>

      <LoginModal
        isOpen={showLogin}
        onClose={() => setShowLogin(false)}
        onLogin={handleLogin}
      />

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
              <div className="flex items-center gap-2">
                {weather && <span className="text-white text-lg font-bold tracking-tight">{weather.temp}°</span>}
                <div className="h-8 w-8 rounded-full bg-primary/20 flex items-center justify-center text-primary">
                  <span className="material-symbols-outlined text-[18px]">{weather ? getWeatherIcon(weather.code) : 'wb_sunny'}</span>
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
            </div>
          </div>
        </section>

        {/* Collections Section (Items List) */}
        <section className="flex flex-col gap-4">
          <div className="flex items-center justify-between px-1">
            <h3 className="text-xl font-bold text-white">Your Feed</h3>
          </div>

          <div className="flex flex-col gap-3">
            {/* Main Feed: Pending Items (Old -> New) */}
            {pendingItems.map((item, index) => {
              const isActive = currentId === item.id;
              // ... (Use same display logic)
              let category = item.category || 'News';
              let displayTitle = item.title;
              if (!item.category) {
                const match = item.title.match(/^【(.*?)】/);
                if (match) category = match[1];
              }
              displayTitle = displayTitle.replace(/^【.*?】/, '').trim();
              const dateRegex = /[-–—]\s*\d{4}[-/]\d{1,2}[-/]\d{1,2}.*?$|\s*\(.*?\d{1,2}:\d{2}.*?\)$/i;
              displayTitle = displayTitle.replace(dateRegex, '').trim();
              const dateObj = item.publish_time ? new Date(item.publish_time * 1000) : new Date();
              const dateStr = dateObj.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
              const timeStr = dateObj.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false });

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
                    <div className={`flex flex-col items-center justify-center rounded-xl size-14 shadow-inner leading-none ${isActive ? 'bg-primary text-black' : 'bg-[#244732] text-white'}`}>
                      {isActive && isPlaying ? (
                        <AnimatedEqualizer size="lg" />
                      ) : (
                        <span className="material-symbols-outlined text-[28px]">graphic_eq</span>
                      )}
                    </div>
                    <div className="absolute -top-1.5 -left-1.5 bg-black/80 backdrop-blur-sm text-white/70 text-[9px] uppercase font-bold px-1.5 py-0.5 rounded-md shadow-sm ring-1 ring-white/10 tracking-wider">
                      {category.substring(0, 4)}
                    </div>
                  </div>
                  <div className="flex flex-col justify-center grow min-w-0">
                    <h4 className={`text-base font-bold leading-tight truncate ${isActive ? 'text-primary' : 'text-white'}`}>
                      {displayTitle}
                    </h4>
                    <p className="text-[#93c8a8] text-xs mt-1 flex items-center gap-3 whitespace-nowrap">
                      <span className="flex items-center opacity-80">
                        <span className="material-symbols-outlined icon-tiny">schedule</span>
                        {dateStr} {timeStr}
                      </span>
                      <span className="flex items-center">
                        <span className="material-symbols-outlined icon-tiny">timer</span>
                        {item.duration_sec ? formatTime(item.duration_sec) : 'Brief'}
                      </span>
                    </p>
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
          </div>
        </section>

      </main>

      {/* Unified Hero Player Widget */}
      {
        currentItem && (
          <>
            {/* Backdrop (Only blocking when expanded) */}
            <div
              className={`fixed inset-0 bg-black/60 backdrop-blur-sm z-40 transition-opacity duration-500 ${isPlayerExpanded ? 'opacity-100 pointer-events-auto' : 'opacity-0 pointer-events-none'}`}
              onClick={() => setIsPlayerExpanded(false)}
            />

            {/* The Morphing Widget */}
            <div
              className={`fixed z-50 transition-all duration-500 ease-[cubic-bezier(0.32,0.72,0,1)] shadow-2xl overflow-hidden ring-1 ring-white/10
                ${isPlayerExpanded
                  ? 'inset-x-0 bottom-0 top-[15vh] rounded-t-[32px] bg-surface-dark' // Expanded Sheet
                  : 'bottom-6 left-4 right-4 h-20 bg-[#1e1e1e] rounded-[32px] max-w-md mx-auto cursor-pointer hover:scale-[1.02]' // Floating Bar
                }
              `}
              onClick={(e) => {
                if (!isPlayerExpanded) {
                  setIsPlayerExpanded(true);
                  e.stopPropagation();
                }
              }}
            >
              {isPlayerExpanded ? (
                // --- EXPANDED MODE ---
                <div className="flex flex-col h-full w-full animate-in fade-in duration-300 bg-surface-dark">
                  {/* Header (No Close Button, No Tags) */}
                  <div className="flex flex-col p-6 pb-2 shrink-0 gap-4">
                    {/* Title Area */}
                    <div className="flex flex-col items-center text-center gap-1 mt-2">
                      <h3 className="text-xl font-bold text-white leading-tight px-4 line-clamp-2">{currentItem.title}</h3>
                      <p className="text-sm text-white/40">{currentItem.category || 'News'}</p>
                    </div>

                    {/* Progress & Controls */}
                    <div className="flex flex-col gap-4 mt-2">
                      {/* Progress */}
                      <div className="flex flex-col gap-1.5 px-4 mb-2">
                        <input
                          type="range"
                          min="0"
                          max={duration || 100}
                          value={progress}
                          onChange={handleSeek}
                          onClick={(e) => e.stopPropagation()}
                          className="w-full h-1.5 bg-white/10 rounded-full appearance-none cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-primary"
                        />
                        <div className="flex justify-between text-[11px] text-white/40 font-mono px-1">
                          <span>{formatTime(progress)}</span>
                          <span>{formatTime(duration)}</span>
                        </div>
                      </div>

                      {/* Main Controls */}
                      <div className="flex items-center justify-center gap-8">
                        <button onClick={(e) => { e.stopPropagation(); skipTime(-15); }} className="text-white/40 hover:text-white transition-colors p-2">
                          <span className="material-symbols-outlined text-[28px]">replay_10</span>
                        </button>
                        <button onClick={(e) => { e.stopPropagation(); playPrev(); }} className="text-white/60 hover:text-white transition-colors p-2">
                          <span className="material-symbols-outlined text-[36px]">skip_previous</span>
                        </button>
                        <button onClick={(e) => { e.stopPropagation(); togglePlay(); }} className="text-white hover:text-primary transition-colors p-2 scale-110">
                          {isBuffering ? (
                            <div className="size-[64px] rounded-full border-4 border-white/20 border-t-primary animate-spin" />
                          ) : (
                            <span className="material-symbols-outlined text-[64px] filled">
                              {isPlaying ? 'pause_circle' : 'play_circle'}
                            </span>
                          )}
                        </button>
                        <button onClick={(e) => { e.stopPropagation(); playNext(); }} className="text-white/60 hover:text-white transition-colors p-2">
                          <span className="material-symbols-outlined text-[36px]">skip_next</span>
                        </button>
                        <button onClick={(e) => { e.stopPropagation(); skipTime(30); }} className="text-white/40 hover:text-white transition-colors p-2">
                          <span className="material-symbols-outlined text-[28px]">forward_30</span>
                        </button>
                      </div>
                    </div>
                  </div>

                  {/* Body Content (Flex Layout to Separate Content and Footer) */}
                  <div className="flex-1 flex flex-col overflow-hidden border-t border-white/5 bg-black/20">
                    <div className="flex-1 overflow-y-auto p-4" onClick={(e) => e.stopPropagation()}>
                      {panelView === 'transcript' ? (
                        // Transcript Content
                        <div className="prose prose-invert prose-lg max-w-none px-2">
                          {currentItem.summary ? (
                            <p className="whitespace-pre-wrap font-serif leading-relaxed text-white/90 text-[1.05rem]">{currentItem.summary}</p>
                          ) : (
                            <div className="py-20 text-center text-white/30">暂无文稿内容</div>
                          )}
                        </div>
                      ) : (
                        // Playlist Content
                        <div className="space-y-4">
                          {pendingItems.length > 0 ? (
                            <div className="space-y-1">
                              {pendingItems.map(item => {
                                const isActive = currentId === item.id;
                                const dateObj = item.publish_time ? new Date(item.publish_time * 1000) : new Date();
                                const dateStr = dateObj.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
                                const timeStr = dateObj.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false });
                                return (
                                  <div key={item.id} onClick={() => playItem(item.id, item.audio_url || '')}
                                    className={`flex items-center gap-3 p-3 rounded-xl cursor-pointer transition-colors ${isActive ? 'bg-primary/10' : 'hover:bg-white/5'}`}>
                                    <div className={`flex items-center justify-center size-10 rounded-lg shrink-0 ${isActive ? 'bg-primary text-black' : 'bg-white/5 text-white/40'}`}>
                                      {isActive && isPlaying ? <AnimatedEqualizer size="sm" /> : <span className="material-symbols-outlined text-xl">graphic_eq</span>}
                                    </div>
                                    <div className="min-w-0 grow">
                                      <h4 className={`text-sm font-bold truncate ${isActive ? 'text-primary' : 'text-white'}`}>{item.title}</h4>
                                      <div className="flex items-center gap-3 text-[10px] text-white/30 mt-0.5 whitespace-nowrap">
                                        <span className="flex items-center"><span className="material-symbols-outlined icon-tiny">schedule</span>{dateStr} {timeStr}</span>
                                        <span className="flex items-center"><span className="material-symbols-outlined icon-tiny">timer</span>{item.duration_sec ? formatTime(item.duration_sec) : '--:--'}</span>
                                      </div>
                                    </div>
                                  </div>
                                )
                              })}
                            </div>
                          ) : <div className="text-center py-4 text-white/30">队列为空</div>}

                          {/* Played Items */}
                          {playedItems.length > 0 && (
                            <div className="border-t border-white/5 mt-4 pt-4">
                              <button onClick={() => setPlayedExpanded(!playedExpanded)} className="flex items-center justify-between w-full px-2 py-2 text-white/40 hover:text-white transition-colors">
                                <span className="text-xs font-bold uppercase tracking-wider">已播放 ({playedItems.length})</span>
                                <span className="material-symbols-outlined">{playedExpanded ? 'expand_less' : 'expand_more'}</span>
                              </button>
                              {playedExpanded && (
                                <div className="space-y-1 mt-2 opacity-60">
                                  {playedItems.slice(0, 20).map(item => (
                                    <div key={item.id} onClick={() => playItem(item.id, item.audio_url || '')} className="flex items-center gap-3 p-3 rounded-xl cursor-pointer hover:bg-white/5">
                                      <span className="material-symbols-outlined text-white/20 text-lg">replay</span>
                                      <h4 className="text-sm font-medium truncate text-white/40">{item.title}</h4>
                                    </div>
                                  ))}
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                      )}
                    </div>

                    {/* Bottom Switcher (Fixed Block) */}
                    <div className="shrink-0 py-4 flex justify-center bg-surface-dark/95 backdrop-blur border-t border-white/5" onClick={(e) => e.stopPropagation()}>
                      <div className="flex items-center gap-1 p-1 bg-[#1a1a1a] rounded-2xl shadow-xl ring-1 ring-white/10">
                        <button onClick={(e) => { e.stopPropagation(); setPanelView('transcript'); }}
                          className={`px-5 py-2.5 rounded-xl text-sm font-bold transition-all ${panelView === 'transcript' ? 'bg-primary text-black shadow-md' : 'text-white/50 hover:text-white'}`}>
                          文稿
                        </button>
                        <button onClick={(e) => { e.stopPropagation(); setPanelView('playlist'); }}
                          className={`px-5 py-2.5 rounded-xl text-sm font-bold transition-all ${panelView === 'playlist' ? 'bg-primary text-black shadow-md' : 'text-white/50 hover:text-white'}`}>
                          列表
                        </button>
                      </div>
                    </div>
                  </div>
                </div>
              ) : (
                // --- COLLAPSED MODE (Floating Bar) ---
                <div className="flex items-center h-full px-2 animate-in fade-in duration-300">
                  {/* Circle Progress/Icon */}
                  <div className="relative size-14 shrink-0 flex items-center justify-center ml-1">
                    <svg className="transform -rotate-90 size-14 drop-shadow-[0_0_8px_rgba(25,230,107,0.3)]">
                      <circle className="text-white/10" cx="28" cy="28" fill="transparent" r="26" stroke="currentColor" strokeWidth="2"></circle>
                      <circle className="text-primary transition-all duration-300" cx="28" cy="28" fill="transparent" r="26" stroke="currentColor"
                        strokeDasharray={2 * Math.PI * 26}
                        strokeDashoffset={(2 * Math.PI * 26) - (progress / (duration || 100)) * (2 * Math.PI * 26)}
                        strokeLinecap="round" strokeWidth="2"></circle>
                    </svg>
                    <div className="absolute inset-0 m-auto size-10 rounded-full bg-surface-highlight overflow-hidden flex items-center justify-center">
                      {isPlaying ? <AnimatedEqualizer size="sm" className="text-primary" /> : <span className="material-symbols-outlined text-white/50 text-xl">graphic_eq</span>}
                    </div>
                  </div>

                  {/* Info */}
                  <div className="flex flex-col ml-3 mr-auto overflow-hidden min-w-0 justify-center">
                    <h4 className="text-white text-sm font-bold truncate pr-4">{currentItem.title}</h4>
                    <p className="text-[#93c8a8] text-xs truncate opacity-80">{formatTime(duration - progress)} remaining</p>
                  </div>

                  {/* Controls */}
                  <div className="flex items-center gap-3 shrink-0 pr-4" onClick={(e) => e.stopPropagation()}>
                    <button onClick={togglePlay} className="text-white hover:text-primary transition-colors flex items-center justify-center scale-110">
                      {isBuffering ? (
                        <div className="size-8 rounded-full border-2 border-white/20 border-t-primary animate-spin" />
                      ) : (
                        <span className="material-symbols-outlined text-[36px] filled">
                          {isPlaying ? 'pause_circle' : 'play_circle'}
                        </span>
                      )}
                    </button>
                    <button onClick={playNext} className="text-white/60 hover:text-white transition-colors flex items-center justify-center">
                      <span className="material-symbols-outlined text-[32px]">skip_next</span>
                    </button>
                  </div>
                </div>
              )}
            </div>
          </>
        )
      }

      {/* Debug Console Overlay */}
      {
        showDebug && (
          debugMinimized ? (
            <div
              className="fixed bottom-24 right-4 z-[100] flex flex-col gap-2 items-end animate-in fade-in slide-in-from-bottom-4"
            >
              <button
                onClick={() => setDebugMinimized(false)}
                className="bg-black/80 backdrop-blur text-green-400 border border-green-500/30 p-3 rounded-full shadow-lg hover:scale-110 transition-transform"
                title="Expand Console"
              >
                <span className="material-symbols-outlined text-xl">terminal</span>
              </button>
            </div>
          ) : (
            <div className="fixed inset-x-0 bottom-0 z-[100] h-[50vh] bg-black/90 text-green-400 font-mono text-[10px] p-2 overflow-y-auto border-t border-white/20 pointer-events-auto shadow-2xl">
              <div className="flex justify-between items-center bg-white/10 p-1 mb-2 rounded sticky top-0 z-10 backdrop-blur-sm">
                <span className="font-bold text-white flex items-center gap-2">
                  <span className="material-symbols-outlined text-sm">terminal</span>
                  Debug Console ({logs.length})
                </span>
                <div className="flex gap-2">
                  <button onClick={() => {
                    const text = logs.join('\n');
                    navigator.clipboard.writeText(text).then(() => alert('Logs copied!'));
                  }} className="px-2 py-1 bg-blue-600/80 hover:bg-blue-600 text-white rounded transition-colors">Copy</button>
                  <button onClick={() => setLogs([])} className="px-2 py-1 bg-white/20 hover:bg-white/30 text-white rounded transition-colors">Clear</button>
                  <button onClick={() => setDebugMinimized(true)} className="px-2 py-1 bg-yellow-600/80 hover:bg-yellow-600 text-white rounded transition-colors flex items-center gap-1">
                    <span className="material-symbols-outlined text-[10px]">minimize</span> Min
                  </button>
                  <button onClick={() => setShowDebug(false)} className="px-2 py-1 bg-red-600/80 hover:bg-red-600 text-white rounded transition-colors">Close</button>
                </div>
              </div>
              <div className="whitespace-pre-wrap break-all px-1 pb-4">
                {logs.map((log, i) => (
                  <div key={i} className="border-b border-white/5 py-0.5 hover:bg-white/5">{log}</div>
                ))}
                <div id="log-end" />
              </div>
            </div>
          )
        )
      }

      {/* Transcript Overlay */}
      {
        showTranscript && (() => {
          const transcriptItem = transcriptItemId
            ? items.find(i => i.id === transcriptItemId) || currentItem
            : currentItem;
          return transcriptItem && (
            <div
              className="fixed inset-0 z-50 flex flex-col items-center justify-end sm:justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200"
              onClick={() => setShowTranscript(false)}
            >
              <div
                className="w-full max-w-2xl bg-surface-dark rounded-3xl shadow-2xl ring-1 ring-white/10 max-h-[80vh] flex flex-col animate-in slide-in-from-bottom-10 duration-200 overscroll-contain"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="flex items-center justify-between p-6 border-b border-white/5 shrink-0 bg-surface-dark/50 z-10 rounded-t-3xl">
                  <div className="pr-4">
                    <p className="text-[#93c8a8] text-xs font-bold uppercase tracking-wider mb-2">文稿</p>
                    <h3 className="text-xl font-bold text-white leading-tight truncate">{transcriptItem.title}</h3>
                  </div>
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
        })()
      }

      {/* Sources Modal */}
      {
        showSources && (
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
        )
      }

    </div >
  );
}

