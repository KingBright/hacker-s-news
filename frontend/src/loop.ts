export type LoopFeedbackMode = "balance" | "boost" | "reduce" | "observe";

export interface LoopReferenceDraft {
  sourceType: string;
  sourceId?: string | null;
  sourceUrl?: string | null;
  title?: string | null;
  quoteText?: string | null;
  startMs?: number | null;
  endMs?: number | null;
}

export interface LoopDraft {
  title?: string | null;
  body?: string | null;
  feedbackMode?: LoopFeedbackMode;
  sourceRef?: string | null;
  references?: LoopReferenceDraft[];
}

export interface LoopPostReference {
  id: string;
  post_id: string;
  source_type: string;
  source_id?: string | null;
  source_url?: string | null;
  title?: string | null;
  quote_text?: string | null;
  start_ms?: number | null;
  end_ms?: number | null;
  created_at?: number | null;
}

export interface LoopPost {
  id: string;
  user_id: string;
  post_type: string;
  feedback_mode?: LoopFeedbackMode | null;
  title?: string | null;
  body: string;
  visibility: string;
  source_ref?: string | null;
  memory_entry_id?: string | null;
  preference_status?: string | null;
  preference_extracted_at?: number | null;
  preference_error?: string | null;
  created_at?: number | null;
  updated_at?: number | null;
  status?: string | null;
}

export interface LoopPostResponse {
  post: LoopPost;
  references: LoopPostReference[];
}

export interface FocusCard {
  label: string;
  kind: string;
  score: number;
  evidence: string;
}

export interface BalanceRule {
  active_pct: number;
  stable_pct: number;
  explore_pct: number;
  note: string;
}

export interface FocusStats {
  expression_count: number;
  processed_expression_count: number;
  pending_expression_count: number;
  signal_count: number;
}

export interface FocusSummary {
  current_focus: FocusCard[];
  long_term_focus: FocusCard[];
  recently_reduced: FocusCard[];
  preferred_sources: FocusCard[];
  preferred_formats: FocusCard[];
  reading_balance: BalanceRule;
  radio_balance: BalanceRule;
  stats: FocusStats;
  note: string;
}

export interface WhyRecommended {
  item_id: string;
  surface: string;
  bucket: string;
  score: number;
  active_score: number;
  stable_score: number;
  explore_score: number;
  reduce_score: number;
  reasons: string[];
  matched_focus: FocusCard[];
  balance: BalanceRule;
  note: string;
}

const LOOP_DRAFT_KEY = "freshloop_loop_draft";

export function loopPreferenceStatusLabel(status?: string | null) {
  switch (status) {
    case "processed":
      return "已吸收";
    case "pending":
      return "待整理";
    case "failed":
      return "整理失败";
    case "skipped":
      return "已略过";
    default:
      return null;
  }
}

export function focusKindLabel(kind?: string | null) {
  switch (kind) {
    case "topic":
      return "主题";
    case "source":
      return "来源";
    case "signal":
      return "偏好";
    case "format":
      return "形态";
    default:
      return kind || "";
  }
}

export function focusBucketLabel(bucket?: string | null) {
  switch (bucket) {
    case "active":
      return "近期焦点";
    case "stable":
      return "长期兴趣";
    case "explore":
      return "探索位";
    default:
      return bucket || "推荐解释";
  }
}

export function saveLoopDraft(draft: LoopDraft) {
  if (typeof window === "undefined") return;
  localStorage.setItem(LOOP_DRAFT_KEY, JSON.stringify(draft));
}

export function loadLoopDraft(): LoopDraft | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = localStorage.getItem(LOOP_DRAFT_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as LoopDraft;
  } catch {
    return null;
  }
}

export function clearLoopDraft() {
  if (typeof window === "undefined") return;
  localStorage.removeItem(LOOP_DRAFT_KEY);
}
