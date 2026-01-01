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
  const [showPlaylist, setShowPlaylist] = useState(false);
  const [isModalOpen, setIsModalOpen] = useState(false);

  const audioRef = useRef<HTMLAudioElement>(null);

  // Poll for new items
  const fetchItems = useCallback(() => {
    fetch('/api/items')
      .then(res => res.json())
      .then(data => {
        // Simple merge: replace list if different length, or if head is different
        // Ideally we should do a smarter merge. For now, just set.
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

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const time = parseFloat(e.target.value);
    setProgress(time);
    if (audioRef.current) {
      audioRef.current.currentTime = time;
    }
  };

  const currentItem = items.find(i => i.id === currentId);

  return (
    <main className="min-h-screen bg-stone-50 text-stone-900 font-sans pb-40">
      <audio
        ref={audioRef}
        onTimeUpdate={handleTimeUpdate}
        onLoadedMetadata={handleLoadedMetadata}
        onEnded={playNext}
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        className="hidden"
      />

      <div className="max-w-3xl mx-auto p-6">
        <header className="mb-8 pt-4 flex justify-between items-end border-b border-stone-200 pb-4 sticky top-0 bg-stone-50/95 backdrop-blur z-10 transition-all">
          <div>
            <h1 className="text-3xl font-serif font-bold tracking-tight text-stone-900">FreshLoop</h1>
            <p className="text-stone-500 text-sm mt-1">Daily Briefing • Zen Mode</p>
          </div>
          <button onClick={fetchItems} className="text-stone-400 hover:text-stone-600 transition p-2">
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" /></svg>
          </button>
        </header>

        <div className="space-y-8">
          {items.map((item, index) => (
            <article
              key={item.id}
              className={`
                group relative p-4 rounded-xl transition-all duration-300
                ${currentId === item.id ? 'bg-white shadow-md ring-1 ring-stone-200 scale-[1.02]' : 'hover:bg-white/50'}
              `}
            >
              <div className="flex items-start gap-4">
                <button
                  onClick={() => item.audio_url && playItem(item.id, item.audio_url)}
                  disabled={!item.audio_url}
                  className={`
                    mt-1 w-10 h-10 flex-shrink-0 flex items-center justify-center rounded-full transition-all
                    ${!item.audio_url ? 'bg-stone-100 text-stone-300 cursor-not-allowed' :
                      currentId === item.id && isPlaying
                        ? 'bg-stone-900 text-white shadow-lg scale-110'
                        : 'bg-stone-200 text-stone-600 hover:bg-stone-800 hover:text-white'}
                  `}
                >
                  {currentId === item.id && isPlaying ? (
                    <svg className="w-4 h-4 fill-current" viewBox="0 0 24 24"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" /></svg>
                  ) : (
                    <svg className="w-4 h-4 fill-current ml-0.5" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
                  )}
                </button>

                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-xs font-bold text-stone-400 font-mono">#{String(items.length - index).padStart(2, '0')}</span>
                    <span className="text-xs text-stone-400">
                      {item.publish_time ? new Date(item.publish_time * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''}
                    </span>
                  </div>

                  <h2 className={`text-xl font-serif font-medium mb-2 leading-tight ${currentId === item.id ? 'text-stone-900' : 'text-stone-700'}`}>
                    <a href={item.original_url || '#'} target="_blank" rel="noopener noreferrer" className="hover:underline decoration-stone-300 underline-offset-4">
                      {item.title}
                    </a>
                  </h2>

                  {item.summary && (
                    <p className={`text-stone-600 leading-relaxed text-sm line-clamp-3 group-hover:line-clamp-none transition-all ${currentId === item.id ? 'line-clamp-none' : ''}`}>
                      {item.summary}
                    </p>
                  )}
                </div>
              </div>
            </article>
          ))}
        </div>
      </div>

      {/* Sticky Player Bar */}
      {currentItem && (
        <div className="fixed bottom-0 left-0 right-0 bg-white/95 backdrop-blur-md border-t border-stone-200 shadow-[0_-4px_20px_rgba(0,0,0,0.05)] pb-safe pt-2 px-4 z-50">
          <div className="max-w-3xl mx-auto">
            {/* Progress Bar */}
            <div className="flex items-center gap-3 mb-2 group">
              <span className="text-xs font-mono text-stone-400 w-10 text-right">{formatTime(progress)}</span>
              <input
                type="range"
                min={0}
                max={duration || 100}
                value={progress}
                onChange={handleSeek}
                onMouseDown={() => setIsDragging(true)}
                onMouseUp={() => setIsDragging(false)}
                onTouchStart={() => setIsDragging(true)}
                onTouchEnd={() => setIsDragging(false)}
                className="
                  flex-1 h-1 bg-stone-200 rounded-lg appearance-none cursor-pointer
                  [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3 
                  [&::-webkit-slider-thumb]:bg-stone-900 [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:transition-transform
                  group-hover:[&::-webkit-slider-thumb]:scale-125
                "
              />
              <span className="text-xs font-mono text-stone-400 w-10">{formatTime(duration)}</span>
            </div>

            <div className="flex items-center justify-between pb-2">
              <div className="flex-1 min-w-0 pr-4">
                <div className="text-xs text-stone-500 uppercase tracking-wider mb-0.5">Now Playing</div>
                <div className="font-serif font-medium text-stone-900 truncate">{currentItem.title}</div>
              </div>

              <div className="flex items-center gap-4 flex-shrink-0">
                <button onClick={playPrev} className="p-2 text-stone-400 hover:text-stone-900 transition">
                  <svg className="w-6 h-6 fill-current" viewBox="0 0 24 24"><path d="M6 6h2v12H6zm3.5 6l8.5 6V6z" /></svg>
                </button>

                <button
                  onClick={togglePlay}
                  className="w-12 h-12 flex items-center justify-center rounded-full bg-stone-900 text-white shadow-lg hover:scale-105 active:scale-95 transition"
                >
                  {isPlaying ? (
                    <svg className="w-5 h-5 fill-current" viewBox="0 0 24 24"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" /></svg>
                  ) : (
                    <svg className="w-5 h-5 fill-current ml-0.5" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
                  )}
                </button>

                <button onClick={playNext} className="p-2 text-stone-400 hover:text-stone-900 transition">
                  <svg className="w-6 h-6 fill-current" viewBox="0 0 24 24"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z" /></svg>
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}
