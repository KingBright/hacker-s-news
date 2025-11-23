"use client";

import { useEffect, useState, useRef } from 'react';
import { Item } from '../src/types';

export default function Home() {
  const [items, setItems] = useState<Item[]>([]);
  const [currentAudio, setCurrentAudio] = useState<string | null>(null);
  const audioRef = useRef<HTMLAudioElement>(null);

  useEffect(() => {
    fetch('http://localhost:8080/api/items')
      .then(res => res.json())
      .then(data => setItems(data))
      .catch(err => console.error('Failed to fetch items:', err));
  }, []);

  const playAudio = (url: string) => {
    if (audioRef.current) {
        if (currentAudio === url && !audioRef.current.paused) {
             audioRef.current.pause();
        } else {
             setCurrentAudio(url);
             // Need to wait for state update or force play
             audioRef.current.src = `http://localhost:8080${url}`;
             audioRef.current.play();
        }
    }
  };

  return (
    <main className="min-h-screen bg-stone-50 text-stone-900 font-sans p-8">
      <div className="max-w-3xl mx-auto">
        <header className="mb-12 border-b border-stone-200 pb-4">
          <h1 className="text-3xl font-serif tracking-tight text-stone-800">FreshLoop <span className="text-sm font-sans text-stone-500 ml-2">Zen Reading</span></h1>
        </header>

        <audio ref={audioRef} onEnded={() => setCurrentAudio(null)} className="hidden" />

        <div className="space-y-12">
          {items.map(item => (
            <article key={item.id} className="group relative">
               <div className="absolute -left-6 top-1 text-stone-300 font-serif text-xl hidden md:block">
                  *
               </div>
              <h2 className="text-2xl font-serif font-medium mb-3 group-hover:text-stone-600 transition-colors">
                <a href={item.original_url || '#'} target="_blank" rel="noopener noreferrer">
                  {item.title}
                </a>
              </h2>
              <div className="text-stone-400 text-sm mb-3">
                 {item.publish_time ? new Date(item.publish_time * 1000).toLocaleDateString() : ''}
              </div>

              {item.summary && (
                <p className="text-stone-700 leading-relaxed text-lg mb-4">
                  {item.summary}
                </p>
              )}

              {item.audio_url && (
                <button
                  onClick={() => playAudio(item.audio_url!)}
                  className={`flex items-center space-x-2 px-4 py-2 rounded-full border transition-all ${currentAudio === item.audio_url ? 'bg-stone-800 text-white border-stone-800' : 'bg-white text-stone-600 border-stone-200 hover:border-stone-400'}`}
                >
                   {currentAudio === item.audio_url ? (
                       <>
                         <span className="w-2 h-2 bg-white rounded-full animate-pulse"></span>
                         <span>Playing...</span>
                       </>
                   ) : (
                       <>
                        <svg className="w-4 h-4 fill-current" viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>
                        <span>Listen</span>
                       </>
                   )}
                </button>
              )}
            </article>
          ))}

          {items.length === 0 && (
              <div className="text-center text-stone-400 py-20">
                  <p>No fresh items yet. The Cortex is thinking...</p>
              </div>
          )}
        </div>
      </div>
    </main>
  );
}
