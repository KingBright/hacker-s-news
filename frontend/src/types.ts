
export interface Item {
  id: string;
  title: string;
  summary: string | null;
  original_url: string | null;
  cover_image_url: string | null;
  audio_url: string | null;
  publish_time: number | null;
  created_at: number | null;
  rating?: number | null;
  tags?: string | null;
  is_deleted?: boolean | null;
  duration_sec?: number | null;
  status?: string;
  category?: string;
}
