"use client";

import { useEffect, useState } from 'react';
import { Item } from '../../src/types';

export default function AdminPage() {
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState(''); // Maps to apiKey
    const [items, setItems] = useState<Item[]>([]);
    const [isAuthenticated, setIsAuthenticated] = useState(false);
    const [loading, setLoading] = useState(false);

    useEffect(() => {
        const storedKey = localStorage.getItem('nexus_key');
        const storedUser = localStorage.getItem('nexus_user'); // Optional persistence
        if (storedKey && storedUser === 'admin') {
            setPassword(storedKey);
            setUsername(storedUser);
            setIsAuthenticated(true);
            fetchItems(storedKey);
        }
    }, []);

    const handleLogin = () => {
        if (username === 'admin' && password) {
            localStorage.setItem('nexus_key', password);
            localStorage.setItem('nexus_user', username);
            setIsAuthenticated(true);
            fetchItems(password);
        } else {
            alert("Invalid Username or Password. (Try user: admin)");
        }
    };

    const fetchItems = (key: string) => {
        setLoading(true);
        // Use public API for listing manifest, but we might eventually need an admin-specific list
        // reusing public list is fine for now as it shows active items.
        fetch('/api/items?limit=100')
            .then(res => res.json())
            .then(data => {
                setItems(data);
                setLoading(false);
            })
            .catch(err => {
                console.error(err);
                setLoading(false);
            });
    };

    const updateItem = async (id: string, updates: any) => {
        try {
            const res = await fetch(`/api/admin/items/${id}`, {
                method: 'PATCH',
                headers: {
                    'Content-Type': 'application/json',
                    'X-NEXUS-KEY': password // Use password state which holds the key
                },
                body: JSON.stringify(updates)
            });

            if (!res.ok) throw new Error('Update failed');

            // Update local state
            setItems(items.map(i => i.id === id ? { ...i, ...updates } : i));

            if (updates.is_deleted) {
                setItems(items.filter(i => i.id !== id));
            }

        } catch (err) {
            alert('Failed to update item: ' + err);
        }
    };

    if (!isAuthenticated) {
        return (
            <div className="min-h-screen flex items-center justify-center bg-background-light dark:bg-background-dark font-display p-4">
                <div className="bg-white dark:bg-surface-dark p-8 rounded-3xl shadow-2xl w-full max-w-md ring-1 ring-black/5 dark:ring-white/10">
                    <div className="mb-8 text-center">
                        <h1 className="text-3xl font-bold text-slate-900 dark:text-white tracking-tight">Admin Access</h1>
                        <p className="text-slate-500 dark:text-[#93c8a8] text-sm font-medium mt-1">FreshLoop Content Management</p>
                    </div>
                    <div className="space-y-5">
                        <div>
                            <label className="block text-xs font-bold text-slate-400 dark:text-[#93c8a8] uppercase tracking-wider mb-1.5 ml-1">Username</label>
                            <input
                                type="text"
                                placeholder="Enter username"
                                className="w-full p-3.5 bg-slate-50 dark:bg-[#0c1610] border-transparent dark:border-white/5 border rounded-xl text-slate-900 dark:text-white placeholder-slate-400 dark:placeholder-white/20 focus:outline-none focus:ring-2 focus:ring-primary focus:bg-white dark:focus:bg-black/40 transition-all font-medium"
                                value={username}
                                onChange={e => setUsername(e.target.value)}
                            />
                        </div>
                        <div>
                            <label className="block text-xs font-bold text-slate-400 dark:text-[#93c8a8] uppercase tracking-wider mb-1.5 ml-1">Password (Nexus Key)</label>
                            <input
                                type="password"
                                placeholder="••••••••"
                                className="w-full p-3.5 bg-slate-50 dark:bg-[#0c1610] border-transparent dark:border-white/5 border rounded-xl text-slate-900 dark:text-white placeholder-slate-400 dark:placeholder-white/20 focus:outline-none focus:ring-2 focus:ring-primary focus:bg-white dark:focus:bg-black/40 transition-all font-medium"
                                value={password}
                                onChange={e => setPassword(e.target.value)}
                            />
                        </div>
                    </div>
                    <button
                        onClick={handleLogin}
                        className="w-full bg-slate-900 dark:bg-primary text-white dark:text-black p-3.5 rounded-full hover:bg-slate-800 dark:hover:opacity-90 transition-all mt-8 font-bold text-lg shadow-lg active:scale-[0.98]"
                    >
                        Access Dashboard
                    </button>
                </div>
            </div>
        );
    }

    return (
        <div className="min-h-screen bg-background-light dark:bg-background-dark text-slate-900 dark:text-white font-display p-6 md:p-10">
            <div className="max-w-7xl mx-auto">
                <header className="flex flex-col md:flex-row justify-between items-start md:items-center mb-10 gap-4">
                    <div>
                        <h1 className="text-4xl font-bold tracking-tight">Content Management</h1>
                        <p className="text-slate-500 dark:text-[#93c8a8] font-medium mt-1">{items.length} active stories</p>
                    </div>
                    <div className="flex gap-3">
                        <button
                            onClick={() => window.open('/api/admin/export', '_blank')}
                            className="bg-slate-200 dark:bg-surface-highlight hover:bg-slate-300 dark:hover:bg-[#2f5c40] px-5 py-2.5 rounded-full text-slate-700 dark:text-white font-semibold transition-colors flex items-center gap-2"
                        >
                            <span className="material-symbols-outlined text-[20px]">download</span>
                            Export Data
                        </button>
                        <button
                            onClick={() => { localStorage.removeItem('nexus_key'); window.location.reload(); }}
                            className="bg-slate-200 dark:bg-surface-highlight hover:bg-red-100 dark:hover:bg-red-900/30 text-slate-700 dark:text-white hover:text-red-600 dark:hover:text-red-400 px-5 py-2.5 rounded-full font-semibold transition-colors"
                        >
                            Logout
                        </button>
                    </div>
                </header>

                {loading ? (
                    <div className="flex flex-col items-center justify-center py-40">
                        <div className="w-10 h-10 border-4 border-primary border-t-transparent rounded-full animate-spin"></div>
                        <p className="mt-4 text-slate-500 dark:text-[#93c8a8] font-medium">Loading contents...</p>
                    </div>
                ) : (
                    <div className="bg-white dark:bg-surface-dark rounded-3xl shadow-sm border border-slate-200 dark:border-white/5 overflow-hidden">
                        <div className="overflow-x-auto">
                            <table className="w-full text-left border-collapse">
                                <thead className="bg-slate-50 dark:bg-black/20 text-slate-500 dark:text-[#93c8a8] text-xs font-bold uppercase tracking-wider">
                                    <tr>
                                        <th className="p-5 font-bold border-b border-slate-200 dark:border-white/5 min-w-[300px]">Title / Audio</th>
                                        <th className="p-5 font-bold border-b border-slate-200 dark:border-white/5 min-w-[300px]">Summary</th>
                                        <th className="p-5 font-bold border-b border-slate-200 dark:border-white/5 w-[140px]">Rating</th>
                                        <th className="p-5 font-bold border-b border-slate-200 dark:border-white/5 min-w-[200px]">Tags</th>
                                        <th className="p-5 font-bold border-b border-slate-200 dark:border-white/5 w-[80px] text-center">Action</th>
                                    </tr>
                                </thead>
                                <tbody className="divide-y divide-slate-100 dark:divide-white/5">
                                    {items.map(item => (
                                        <tr key={item.id} className="hover:bg-slate-50 dark:hover:bg-white/[0.02] transition-colors group">
                                            <td className="p-5 align-top">
                                                <div className="font-bold text-slate-900 dark:text-white text-lg mb-1 leading-snug">{item.title}</div>
                                                <div className="flex items-center gap-2 mb-3">
                                                    <span className="text-xs font-mono text-slate-400 dark:text-white/40 bg-slate-100 dark:bg-white/5 px-1.5 py-0.5 rounded">
                                                        {new Date((item.publish_time || 0) * 1000).toLocaleString()}
                                                    </span>
                                                </div>
                                                {item.audio_url && (
                                                    <div className="bg-slate-100 dark:bg-black/30 rounded-lg p-1.5 inline-block">
                                                        <audio controls src={item.audio_url} className="h-8 max-w-[240px]" />
                                                    </div>
                                                )}
                                            </td>
                                            <td className="p-5 align-top">
                                                <p className="text-sm text-slate-600 dark:text-slate-300 leading-relaxed max-w-prose">{item.summary}</p>
                                            </td>
                                            <td className="p-5 align-top">
                                                <div className="flex gap-1 bg-slate-100 dark:bg-black/20 p-1.5 rounded-lg w-fit">
                                                    {[1, 2, 3, 4, 5].map(star => (
                                                        <button
                                                            key={star}
                                                            onClick={() => updateItem(item.id, { rating: star })}
                                                            className={`w-5 h-5 transition-all duration-200 ${(item.rating || 0) >= star
                                                                    ? 'text-yellow-400 scale-110 drop-shadow-sm'
                                                                    : 'text-slate-300 dark:text-white/10 hover:text-slate-400 dark:hover:text-white/30'
                                                                }`}
                                                        >
                                                            ★
                                                        </button>
                                                    ))}
                                                </div>
                                            </td>
                                            <td className="p-5 align-top">
                                                <input
                                                    type="text"
                                                    placeholder="Add tags..."
                                                    className="w-full p-2.5 text-sm bg-slate-50 dark:bg-black/20 border border-transparent dark:border-white/5 rounded-lg focus:ring-1 focus:ring-primary focus:bg-white dark:focus:bg-black/40 outline-none text-slate-900 dark:text-white transition-all placeholder-slate-400 dark:placeholder-white/20"
                                                    defaultValue={item.tags || ''}
                                                    onBlur={(e) => {
                                                        if (e.target.value !== item.tags) {
                                                            updateItem(item.id, { tags: e.target.value });
                                                        }
                                                    }}
                                                />
                                            </td>
                                            <td className="p-5 align-top text-center">
                                                <button
                                                    onClick={() => {
                                                        if (confirm('Are you sure you want to delete this item?')) {
                                                            updateItem(item.id, { is_deleted: true });
                                                        }
                                                    }}
                                                    className="text-slate-400 dark:text-white/20 hover:text-red-500 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/20 p-2.5 rounded-full transition-all"
                                                    title="Delete"
                                                >
                                                    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" /></svg>
                                                </button>
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
}
