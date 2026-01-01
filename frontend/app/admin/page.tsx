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
            <div className="min-h-screen flex items-center justify-center bg-stone-50">
                <div className="bg-white p-8 rounded-xl shadow-lg w-full max-w-md">
                    <h1 className="text-2xl font-serif font-bold mb-6 text-center">Admin Login</h1>
                    <div className="space-y-4">
                        <div>
                            <label className="block text-sm font-medium text-stone-700 mb-1">Username</label>
                            <input
                                type="text"
                                placeholder="admin"
                                className="w-full p-3 border border-stone-200 rounded-lg focus:ring-2 focus:ring-stone-900 outline-none"
                                value={username}
                                onChange={e => setUsername(e.target.value)}
                            />
                        </div>
                        <div>
                            <label className="block text-sm font-medium text-stone-700 mb-1">Password (Nexus Key)</label>
                            <input
                                type="password"
                                placeholder="••••••••"
                                className="w-full p-3 border border-stone-200 rounded-lg focus:ring-2 focus:ring-stone-900 outline-none"
                                value={password}
                                onChange={e => setPassword(e.target.value)}
                            />
                        </div>
                    </div>
                    <button
                        onClick={handleLogin}
                        className="w-full bg-stone-900 text-white p-3 rounded-lg hover:bg-stone-800 transition mt-6 font-medium"
                    >
                        Access Dashboard
                    </button>
                </div>
            </div>
        );
    }

    return (
        <div className="min-h-screen bg-stone-50 p-8 font-sans">
            <div className="max-w-6xl mx-auto">
                <header className="flex justify-between items-center mb-8">
                    <h1 className="text-3xl font-serif font-bold">Content Management</h1>
                    <div className="flex gap-4">
                        <button
                            onClick={() => window.open('/api/admin/export', '_blank')}
                            className="bg-stone-200 hover:bg-stone-300 px-4 py-2 rounded-lg text-stone-700 font-medium transition"
                        >
                            Export High Quality Data
                        </button>
                        <button
                            onClick={() => { localStorage.removeItem('nexus_key'); window.location.reload(); }}
                            className="text-stone-500 hover:text-stone-800"
                        >
                            Logout
                        </button>
                    </div>
                </header>

                {loading ? (
                    <div className="text-center py-20 text-stone-400">Loading...</div>
                ) : (
                    <div className="bg-white rounded-xl shadow-sm border border-stone-200 overflow-hidden">
                        <table className="w-full text-left border-collapse">
                            <thead className="bg-stone-100 text-stone-500 text-sm uppercase tracking-wider">
                                <tr>
                                    <th className="p-4 font-medium border-b border-stone-200 w-1/4">Title / Audio</th>
                                    <th className="p-4 font-medium border-b border-stone-200 w-1/4">Summary</th>
                                    <th className="p-4 font-medium border-b border-stone-200 w-1/6">Rating (1-5)</th>
                                    <th className="p-4 font-medium border-b border-stone-200 w-1/4">Tags</th>
                                    <th className="p-4 font-medium border-b border-stone-200 w-20">Actions</th>
                                </tr>
                            </thead>
                            <tbody className="divide-y divide-stone-100">
                                {items.map(item => (
                                    <tr key={item.id} className="hover:bg-stone-50/50 transition">
                                        <td className="p-4 align-top">
                                            <div className="font-medium text-stone-900 mb-1">{item.title}</div>
                                            <div className="text-xs text-stone-400 mb-2">{new Date((item.publish_time || 0) * 1000).toLocaleString()}</div>
                                            {item.audio_url && (
                                                <audio controls src={item.audio_url} className="h-8 w-full max-w-[200px]" />
                                            )}
                                        </td>
                                        <td className="p-4 align-top">
                                            <p className="text-sm text-stone-600 line-clamp-4">{item.summary}</p>
                                        </td>
                                        <td className="p-4 align-top">
                                            <div className="flex gap-1">
                                                {[1, 2, 3, 4, 5].map(star => (
                                                    <button
                                                        key={star}
                                                        onClick={() => updateItem(item.id, { rating: star })}
                                                        className={`w-6 h-6 rounded-full transition ${(item.rating || 0) >= star ? 'text-yellow-400' : 'text-stone-200 hover:text-stone-300'
                                                            }`}
                                                    >
                                                        ★
                                                    </button>
                                                ))}
                                            </div>
                                        </td>
                                        <td className="p-4 align-top">
                                            <input
                                                type="text"
                                                placeholder="Add tags..."
                                                className="w-full p-2 text-sm border border-stone-200 rounded-md focus:ring-1 focus:ring-stone-400 outline-none"
                                                defaultValue={item.tags || ''}
                                                onBlur={(e) => {
                                                    if (e.target.value !== item.tags) {
                                                        updateItem(item.id, { tags: e.target.value });
                                                    }
                                                }}
                                            />
                                        </td>
                                        <td className="p-4 align-top text-center">
                                            <button
                                                onClick={() => {
                                                    if (confirm('Are you sure you want to delete this item?')) {
                                                        updateItem(item.id, { is_deleted: true });
                                                    }
                                                }}
                                                className="text-red-400 hover:text-red-600 hover:bg-red-50 p-2 rounded transition"
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
                )}
            </div>
        </div>
    );
}
