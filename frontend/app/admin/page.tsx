"use client";

import { useEffect, useState } from 'react';
import { Item } from '../../src/types';

export default function AdminPage() {
    // Auth & Content State
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState(''); // Maps to apiKey
    const [items, setItems] = useState<Item[]>([]);
    const [isAuthenticated, setIsAuthenticated] = useState(false);
    const [loading, setLoading] = useState(false);

    const [users, setUsers] = useState<any[]>([]);
    const [newUserStart, setNewUserStart] = useState('');
    const [createdCreds, setCreatedCreds] = useState<{ username: string, password: string } | null>(null);

    useEffect(() => {
        const storedKey = localStorage.getItem('nexus_key');
        const storedUser = localStorage.getItem('nexus_user'); // Optional persistence
        if (storedKey && storedUser === 'admin') {
            setPassword(storedKey);
            setUsername(storedUser);
            setIsAuthenticated(true);
            fetchItems(storedKey);
            fetchUsers(storedKey);
        }
    }, []);

    const handleLogin = () => {
        if (username === 'admin' && password) {
            localStorage.setItem('nexus_key', password);
            localStorage.setItem('nexus_user', username);
            setIsAuthenticated(true);
            fetchItems(password);
            fetchUsers(password);
        } else {
            alert("Invalid Username or Password. (Try user: admin)");
        }
    };

    const fetchUsers = (key: string) => {
        fetch('/api/admin/users', {
            headers: { 'X-API-KEY': key }
        })
            .then(res => res.json())
            .then(data => setUsers(data || []))
            .catch(e => console.error("Failed to fetch users", e));
    };

    const createUser = async () => {
        if (!newUserStart) return;
        try {
            const res = await fetch('/api/admin/users', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'X-API-KEY': password
                },
                body: JSON.stringify({ username: newUserStart })
            });
            if (!res.ok) throw new Error("Failed");
            const data = await res.json();
            setCreatedCreds({ username: data.username, password: data.password_generated });
            setNewUserStart('');
            fetchUsers(password);
        } catch (e) {
            alert("Failed to create user");
        }
    };

    const fetchItems = (key: string) => {
        setLoading(true);
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

        } catch (err: any) {
            alert('Failed to update item: ' + err);
        }
    };

    const handleRegenerate = async (id: string) => {
        if (!confirm('Regenerate audio for this item? This will overwrite the current files.')) return;

        try {
            const res = await fetch(`/api/admin/items/${id}/regenerate`, {
                method: 'POST',
                headers: {
                    'X-NEXUS-KEY': password
                }
            });

            if (!res.ok) throw new Error('Regeneration request failed');

            alert('Regeneration started. Please allow a few minutes for processing.');
        } catch (err) {
            alert('Failed to start regeneration: ' + err);
        }
    };

    return (
        <div className="min-h-screen bg-background-light dark:bg-background-dark text-slate-900 dark:text-white font-display p-6 md:p-10">
            <div className="max-w-7xl mx-auto">
                <header className="flex flex-col md:flex-row justify-between items-start md:items-center mb-10 gap-4">
                    <div>
                        <h1 className="text-4xl font-bold tracking-tight">Admin Dashboard</h1>
                        <p className="text-slate-500 dark:text-[#93c8a8] font-medium mt-1">Manage Users & Content</p>
                    </div>
                    <div className="flex gap-3">
                        <button
                            onClick={() => { localStorage.removeItem('nexus_key'); window.location.reload(); }}
                            className="bg-slate-200 dark:bg-surface-highlight hover:bg-red-100 dark:hover:bg-red-900/30 text-slate-700 dark:text-white hover:text-red-600 dark:hover:text-red-400 px-5 py-2.5 rounded-full font-semibold transition-colors"
                        >
                            Logout
                        </button>
                    </div>
                </header>

                {/* User Management Section */}
                <div className="mb-12 bg-white dark:bg-surface-dark rounded-3xl p-8 border border-slate-200 dark:border-white/5 shadow-sm">
                    <h2 className="text-2xl font-bold mb-6">User Management</h2>

                    <div className="flex flex-wrap gap-8">
                        {/* Users List */}
                        <div className="flex-1 min-w-[300px]">
                            <h3 className="text-sm font-bold uppercase text-slate-400 dark:text-[#93c8a8] mb-4">Existing Users</h3>
                            <div className="space-y-2 max-h-60 overflow-y-auto pr-2">
                                {users.map(u => (
                                    <div key={u.id} className="flex items-center justify-between p-3 bg-slate-50 dark:bg-black/20 rounded-xl">
                                        <div className="flex items-center gap-3">
                                            <div className="w-8 h-8 rounded-full bg-indigo-500 text-white flex items-center justify-center font-bold text-sm">
                                                {u.username[0].toUpperCase()}
                                            </div>
                                            <div>
                                                <div className="font-bold">{u.username}</div>
                                                <div className="text-xs text-slate-500 dark:text-white/40">ID: {u.id.substring(0, 8)}...</div>
                                            </div>
                                        </div>
                                        <div className="text-xs text-slate-400">
                                            {new Date(u.created_at * 1000).toLocaleDateString()}
                                        </div>
                                    </div>
                                ))}
                                {users.length === 0 && <p className="text-slate-400 italic">No users found.</p>}
                            </div>
                        </div>

                        {/* Create User */}
                        <div className="flex-1 min-w-[300px] border-l border-slate-100 dark:border-white/5 pl-8">
                            <h3 className="text-sm font-bold uppercase text-slate-400 dark:text-[#93c8a8] mb-4">Create New User</h3>
                            <div className="flex gap-2 mb-4">
                                <input
                                    type="text"
                                    className="flex-1 p-3 bg-slate-50 dark:bg-black/20 rounded-xl outline-none focus:ring-2 focus:ring-primary"
                                    placeholder="Username"
                                    value={newUserStart}
                                    onChange={e => setNewUserStart(e.target.value)}
                                />
                                <button
                                    onClick={createUser}
                                    disabled={!newUserStart}
                                    className="bg-indigo-600 hover:bg-indigo-500 text-white px-6 rounded-xl font-bold transition-colors disabled:opacity-50"
                                >
                                    Create
                                </button>
                            </div>

                            {createdCreds && (
                                <div className="bg-green-500/10 border border-green-500/20 p-4 rounded-xl">
                                    <div className="text-green-600 dark:text-green-400 font-bold text-sm mb-1">User Created Successfully!</div>
                                    <div className="text-xs text-slate-500 dark:text-white/60 mb-2">Copy these credentials immediately:</div>
                                    <div className="bg-white dark:bg-black/40 p-3 rounded-lg font-mono text-sm break-all select-all flex flex-col gap-1">
                                        <div>user: <span className="font-bold text-indigo-400">{createdCreds.username}</span></div>
                                        <div>pass: <span className="font-bold text-indigo-400">{createdCreds.password}</span></div>
                                    </div>
                                </div>
                            )}
                        </div>
                    </div>
                </div>

                <div className="flex items-center justify-between mb-6">
                    <div>
                        <h2 className="text-2xl font-bold">Content Library</h2>
                        <p className="text-slate-500 dark:text-[#93c8a8]">{items.length} items</p>
                    </div>
                    <button
                        onClick={() => window.open('/api/admin/export', '_blank')}
                        className="bg-slate-200 dark:bg-surface-highlight hover:bg-slate-300 dark:hover:bg-[#2f5c40] px-5 py-2.5 rounded-full text-slate-700 dark:text-white font-semibold transition-colors flex items-center gap-2"
                    >
                        <span className="material-symbols-outlined text-[20px]">download</span>
                        Export Data
                    </button>
                </div>

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
                                                    onClick={() => handleRegenerate(item.id)}
                                                    className="text-slate-400 dark:text-white/20 hover:text-blue-500 dark:hover:text-blue-400 hover:bg-blue-50 dark:hover:bg-blue-900/20 p-2.5 rounded-full transition-all mr-1"
                                                    title="Regenerate"
                                                >
                                                    <span className="material-symbols-outlined text-[20px]">refresh</span>
                                                </button>
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
