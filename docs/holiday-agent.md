# FreshLoop Holiday Agent

FreshLoop Radio uses local structured holiday data when writing date-aware audio
openings. It does not fetch holiday pages during episode generation.

## Data Flow

1. `cortex holiday-agent --url <official-notice-url>` downloads an official
   holiday notice.
2. The agent extracts holiday ranges and adjusted workdays into JSON.
3. Cortex reads `~/.freshloop/cache/holiday_calendar.json` first, then falls
   back to the bundled `backend/cortex/data/holiday_calendar.json`.
4. If a future year has no official JSON yet, Cortex uses conservative rule
   projections and labels them as `规则推算` in the prompt.

## Usage

```bash
cortex holiday-agent \
  --url "https://www.gov.cn/..." \
  --output "$HOME/.freshloop/cache/holiday_calendar.json"
```

Multiple `--url` values can be passed in one run. If the same year already
exists, the latest extracted result replaces that year.

For automation, set:

```bash
export FRESHLOOP_HOLIDAY_NOTICE_URLS="https://www.gov.cn/notice-a.html,https://www.gov.cn/notice-b.html"
cortex holiday-agent
```

## Safety Boundary

Official holiday notices override projections. Projections are only a fallback
for future years before the official notice is ingested, and they never create
adjusted workdays.
