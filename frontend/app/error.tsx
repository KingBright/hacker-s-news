"use client";

import { useEffect } from 'react';

interface ErrorProps {
    error: Error & { digest?: string };
    reset: () => void;
}

export default function Error({ error, reset }: ErrorProps) {
    useEffect(() => {
        // Log the error to console for debugging
        console.error('Global error caught:', error);
    }, [error]);

    return (
        <div className="min-h-screen bg-background-dark flex items-center justify-center p-4">
            <div className="bg-surface-dark rounded-3xl shadow-xl p-8 w-full max-w-md border border-white/5 text-center">
                <div className="w-16 h-16 bg-red-500/20 rounded-full flex items-center justify-center mx-auto mb-6">
                    <span className="material-symbols-outlined text-4xl text-red-400">error</span>
                </div>

                <h2 className="text-2xl font-bold text-white mb-2">Something went wrong</h2>
                <p className="text-white/60 mb-6">
                    An unexpected error occurred. Please try again.
                </p>

                {error.message && (
                    <div className="bg-black/30 rounded-lg p-3 mb-6 text-left">
                        <p className="text-xs text-white/40 font-mono break-all">
                            {error.message}
                        </p>
                    </div>
                )}

                <div className="flex gap-3 justify-center">
                    <button
                        onClick={() => reset()}
                        className="bg-primary hover:bg-primary/80 text-black font-bold py-3 px-6 rounded-xl transition-colors"
                    >
                        Try again
                    </button>
                    <button
                        onClick={() => window.location.href = '/'}
                        className="bg-white/10 hover:bg-white/20 text-white font-bold py-3 px-6 rounded-xl transition-colors"
                    >
                        Go home
                    </button>
                </div>
            </div>
        </div>
    );
}
