
export interface Item {
  id: string;
  title: string;
  summary: string | null;
  original_url: string | null;
  cover_image_url: string | null;
  audio_url: string | null;
  publish_time: number | null;
  created_at: number | null;
}
