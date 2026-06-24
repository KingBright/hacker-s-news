export interface DayPlaylistGroup<T> {
  key: string;
  startMs: number;
  title: string;
  shortTitle: string;
  items: T[];
  itemIds: string[];
  playbackIds: string[];
  playableCount: number;
  totalDurationSec: number;
}

type SortOrder = "asc" | "desc";

interface BuildDayPlaylistsOptions<T> {
  items: T[];
  getId: (item: T) => string;
  getTimestampMs: (item: T) => number | null | undefined;
  isPlayable?: (item: T) => boolean;
  getDurationSec?: (item: T) => number | null | undefined;
  dayOrder?: SortOrder;
  itemOrder?: SortOrder;
  playbackOrder?: SortOrder;
  now?: Date;
}

interface DayBucket<T> {
  key: string;
  startMs: number;
  items: T[];
}

function compareNumbers(left: number, right: number, order: SortOrder) {
  return order === "asc" ? left - right : right - left;
}

function localDayParts(date: Date) {
  return {
    year: date.getFullYear(),
    month: date.getMonth() + 1,
    day: date.getDate(),
  };
}

function dayKeyFromDate(date: Date) {
  const { year, month, day } = localDayParts(date);
  return `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function localDayStartMs(timestampMs: number) {
  const date = new Date(timestampMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function formatDayTitle(startMs: number, now: Date) {
  const target = new Date(startMs);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const yesterday = today - 24 * 60 * 60 * 1000;
  const shortFormatter = new Intl.DateTimeFormat("zh-CN", {
    month: "short",
    day: "numeric",
  });
  const fullFormatter = new Intl.DateTimeFormat("zh-CN", {
    month: "long",
    day: "numeric",
    weekday: "short",
  });
  const absoluteShort = shortFormatter.format(target);
  const absoluteFull = fullFormatter.format(target);

  if (startMs === today) {
    return { title: `今天 · ${absoluteFull}`, shortTitle: "今天" };
  }
  if (startMs === yesterday) {
    return { title: `昨天 · ${absoluteFull}`, shortTitle: "昨天" };
  }
  return { title: absoluteFull, shortTitle: absoluteShort };
}

export function buildDayPlaylists<T>({
  items,
  getId,
  getTimestampMs,
  isPlayable = () => true,
  getDurationSec = () => 0,
  dayOrder = "desc",
  itemOrder = "asc",
  playbackOrder = "asc",
  now = new Date(),
}: BuildDayPlaylistsOptions<T>): DayPlaylistGroup<T>[] {
  const buckets = new Map<string, DayBucket<T>>();

  for (const item of items) {
    const rawTimestampMs = getTimestampMs(item) ?? 0;
    const timestampMs = rawTimestampMs > 0 ? rawTimestampMs : 0;
    const startMs = timestampMs > 0 ? localDayStartMs(timestampMs) : localDayStartMs(now.getTime());
    const key = dayKeyFromDate(new Date(startMs));
    const bucket = buckets.get(key);

    if (bucket) {
      bucket.items.push(item);
      continue;
    }

    buckets.set(key, {
      key,
      startMs,
      items: [item],
    });
  }

  return Array.from(buckets.values())
    .sort((left, right) => compareNumbers(left.startMs, right.startMs, dayOrder))
    .map((bucket) => {
      const orderedItems = [...bucket.items].sort((left, right) => {
        const leftTs = getTimestampMs(left) ?? 0;
        const rightTs = getTimestampMs(right) ?? 0;
        const byTime = compareNumbers(leftTs, rightTs, itemOrder);
        if (byTime !== 0) return byTime;
        return getId(left).localeCompare(getId(right));
      });

      const playbackIds = [...bucket.items]
        .filter((item) => isPlayable(item))
        .sort((left, right) => {
          const leftTs = getTimestampMs(left) ?? 0;
          const rightTs = getTimestampMs(right) ?? 0;
          const byTime = compareNumbers(leftTs, rightTs, playbackOrder);
          if (byTime !== 0) return byTime;
          return getId(left).localeCompare(getId(right));
        })
        .map((item) => getId(item));

      const labels = formatDayTitle(bucket.startMs, now);
      return {
        key: bucket.key,
        startMs: bucket.startMs,
        title: labels.title,
        shortTitle: labels.shortTitle,
        items: orderedItems,
        itemIds: orderedItems.map((item) => getId(item)),
        playbackIds,
        playableCount: playbackIds.length,
        totalDurationSec: bucket.items.reduce((total, item) => {
          const duration = getDurationSec(item) ?? 0;
          return total + (duration > 0 ? duration : 0);
        }, 0),
      };
    });
}
