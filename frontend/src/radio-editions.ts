import type { Item } from './types';
import type { DayPlaylistGroup } from './day-playlists';

export function radioEdition(item: Item) {
  let tags: string[] = [];
  try { const parsed = JSON.parse(item.tags || '[]'); if (Array.isArray(parsed)) tags = parsed.filter((v): v is string => typeof v === 'string'); } catch { /* Legacy tags are optional. */ }
  const local = new Date(((item.publish_time || item.created_at || 0) + 8 * 3600) * 1000);
  const date = tags.find(t => /^radio:date:\d{4}-\d{2}-\d{2}$/.test(t))?.slice(11) || local.toISOString().slice(0, 10);
  const edition = tags.includes('radio:edition:morning') ? 'morning' : tags.includes('radio:edition:evening') ? 'evening' : local.getUTCHours() < 12 ? 'morning' : 'evening';
  return { key: `${date}:${edition}`, date, edition };
}

export function buildRadioEditions(items: Item[]): DayPlaylistGroup<Item>[] {
  const buckets = new Map<string, Item[]>();
  for (const item of items) {
    const { key } = radioEdition(item);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(item); else buckets.set(key, [item]);
  }
  return [...buckets.entries()].map(([key, members]) => {
    const { date, edition } = radioEdition(members[0]);
    const startMs = Date.parse(`${date}T00:00:00+08:00`);
    const label = new Intl.DateTimeFormat('zh-CN', { timeZone: 'Asia/Shanghai', year: 'numeric', month: 'long', day: 'numeric', weekday: 'long' }).format(new Date(startMs));
    const programs = members.filter(i => isRadioProgram(i) && i.audio_url?.trim());
    const ordered = [...(programs.length ? programs : members)].sort((a,b) => (a.publish_time || a.created_at || 0) - (b.publish_time || b.created_at || 0) || a.id.localeCompare(b.id));
    const playable = ordered.filter(i => i.audio_url?.trim());
    return {key, startMs, title: `${edition === 'morning' ? '早间新闻' : '晚间新闻'} · ${label}`, shortTitle: edition === 'morning' ? '早间新闻' : '晚间新闻', items: ordered, itemIds: ordered.map(i=>i.id), playbackIds: playable.map(i=>i.id), playableCount: playable.length, totalDurationSec: playable.reduce((n,i)=>n+Math.max(0,i.duration_sec || 0),0)};
  }).sort((a,b)=>b.startMs-a.startMs || a.key.localeCompare(b.key));
}

export function isRadioProgram(item: Item): boolean {
  try { const tags = JSON.parse(item.tags || '[]'); return Array.isArray(tags) && tags.includes('radio:program'); } catch { return false; }
}

export function programSectionCount(item: Item): number {
  try { const tags = JSON.parse(item.tags || '[]'); return Array.isArray(tags) ? Number(tags.find((t: unknown) => typeof t === 'string' && /^radio:sections:\d+$/.test(t))?.split(':')[2] || 0) : 0; } catch { return 0; }
}
