"use client";

import { useState, useEffect } from "react";

interface LoginModalProps {
    isOpen: boolean;
    onClose: () => void;
    onLogin: (user: { id: string; username: string }) => void;
}

export function LoginModal({ isOpen, onClose, onLogin }: LoginModalProps) {
    const [username, setUsername] = useState("");
    const [password, setPassword] = useState("");
    const [error, setError] = useState("");
    const [loading, setLoading] = useState(false);

    useEffect(() => {
        if (isOpen) {
            setError("");
            setUsername("");
            setPassword("");
        }
    }, [isOpen]);

    const handleLogin = async () => {
        setLoading(true);
        setError("");
        try {
            const res = await fetch("/api/auth/login", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ username, password }),
            });

            if (!res.ok) throw new Error("Authentication failed");

            const user = await res.json();
            onLogin(user);
            onClose();
        } catch (e) {
            setError("Login failed. Check credentials.");
        } finally {
            setLoading(false);
        }
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4">
            <div className="bg-zinc-900 border border-zinc-800 rounded-2xl w-full max-w-sm shadow-2xl p-6 relative">
                <button
                    onClick={onClose}
                    className="absolute top-4 right-4 text-zinc-500 hover:text-white"
                >
                    <span className="material-symbols-outlined">close</span>
                </button>

                <h2 className="text-xl font-bold text-white mb-1 flex items-center gap-2">
                    <span className="material-symbols-outlined text-indigo-400">lock</span>
                    Sign In
                </h2>
                <p className="text-zinc-400 text-xs mb-6 ml-8">
                    登录后就可以同步收听状态哦
                </p>

                {error && (
                    <div className="bg-red-500/10 text-red-400 text-sm p-3 rounded-lg mb-4">
                        {error}
                    </div>
                )}

                <div className="space-y-4">
                    <div>
                        <label className="block text-xs font-medium text-zinc-400 mb-1">Username</label>
                        <input
                            type="text"
                            value={username}
                            onChange={(e) => setUsername(e.target.value)}
                            className="w-full bg-zinc-800 border border-zinc-700 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                            placeholder="Enter username"
                        />
                    </div>
                    <div>
                        <label className="block text-xs font-medium text-zinc-400 mb-1">Password</label>
                        <input
                            type="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            className="w-full bg-zinc-800 border border-zinc-700 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                            placeholder="Enter password"
                        />
                    </div>

                    <button
                        onClick={handleLogin}
                        disabled={loading}
                        className="w-full bg-indigo-600 hover:bg-indigo-500 text-white font-medium py-2 rounded-lg transition-colors flex items-center justify-center gap-2"
                    >
                        {loading ? "Signing in..." : "Sign In"}
                    </button>

                    <div className="mt-4 pt-4 border-t border-zinc-800 text-center">
                        <p className="text-xs text-zinc-500">
                            没有账号？请联系管理员创建用户
                        </p>
                    </div>
                </div>
            </div>
        </div>
    );
}
