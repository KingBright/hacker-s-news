use anyhow::{Context, Result};
use chinese_lunisolar_calendar::LunisolarDate;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const BUNDLED_CALENDAR: &str = include_str!("../../data/holiday_calendar.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolidayMatch {
    pub name: String,
    pub certainty: HolidayCertainty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolidayCertainty {
    Official,
    Projected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HolidayCalendarFile {
    pub schema_version: u8,
    pub generated_at: String,
    #[serde(default)]
    pub years: Vec<HolidayYear>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HolidayYear {
    pub year: i32,
    pub source_title: String,
    pub source_url: String,
    #[serde(default)]
    pub holidays: Vec<HolidayPeriod>,
    #[serde(default)]
    pub adjusted_workdays: Vec<AdjustedWorkday>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HolidayPeriod {
    pub name: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdjustedWorkday {
    pub date: String,
    pub name: String,
}

pub fn lookup_holiday(date: NaiveDate) -> Option<HolidayMatch> {
    let runtime_calendar = load_runtime_calendar().ok();
    if let Some(calendar) = runtime_calendar.as_ref() {
        if let Some(name) = lookup_official_holiday(calendar, date) {
            return Some(HolidayMatch {
                name,
                certainty: HolidayCertainty::Official,
            });
        }
    }

    if let Ok(calendar) = bundled_calendar() {
        if let Some(name) = lookup_official_holiday(&calendar, date) {
            return Some(HolidayMatch {
                name,
                certainty: HolidayCertainty::Official,
            });
        }
    }

    projected_holiday(date).map(|name| HolidayMatch {
        name,
        certainty: HolidayCertainty::Projected,
    })
}

pub fn lookup_adjusted_workday(date: NaiveDate) -> Option<String> {
    let runtime_calendar = load_runtime_calendar().ok();
    if let Some(calendar) = runtime_calendar.as_ref() {
        if let Some(name) = lookup_official_adjusted_workday(calendar, date) {
            return Some(name);
        }
    }

    bundled_calendar()
        .ok()
        .and_then(|calendar| lookup_official_adjusted_workday(&calendar, date))
}

pub fn official_calendar_known_for_year(year: i32) -> bool {
    let runtime_calendar = load_runtime_calendar().ok();
    if runtime_calendar
        .as_ref()
        .is_some_and(|calendar| calendar.years.iter().any(|entry| entry.year == year))
    {
        return true;
    }

    bundled_calendar()
        .ok()
        .is_some_and(|calendar| calendar.years.iter().any(|entry| entry.year == year))
}

pub fn traditional_festival_names(date: NaiveDate) -> Vec<&'static str> {
    let mut names = Vec::new();
    match (date.month(), date.day()) {
        (1, 1) => names.push("元旦"),
        (5, 1) => names.push("劳动节"),
        (10, 1) => names.push("国庆节"),
        _ => {}
    }

    if let Ok(lunar) = LunisolarDate::from_date(date) {
        let lunar_month = lunar.to_lunar_month().to_u8();
        let lunar_day = lunar.to_lunar_day().to_u8();
        match (lunar_month, lunar_day) {
            (1, 1) => names.push("春节"),
            (1, 15) => names.push("元宵节"),
            (5, 5) => names.push("端午节"),
            (8, 15) => names.push("中秋节"),
            _ => {}
        }
    }

    names
}

pub async fn run_holiday_agent<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let mut urls = Vec::new();
    let mut output_path = default_runtime_calendar_path();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--url" => {
                let url = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("holiday-agent --url missing value"))?;
                urls.push(url);
            }
            "--output" => {
                output_path = PathBuf::from(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("holiday-agent --output missing value"))?,
                );
            }
            "--help" | "-h" => {
                print_holiday_agent_usage();
                return Ok(());
            }
            unknown => anyhow::bail!("unknown holiday-agent argument: {}", unknown),
        }
    }

    if urls.is_empty() {
        if let Ok(env_urls) = std::env::var("FRESHLOOP_HOLIDAY_NOTICE_URLS") {
            urls.extend(
                env_urls
                    .split(',')
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(ToOwned::to_owned),
            );
        }
    }

    if urls.is_empty() {
        anyhow::bail!(
            "holiday-agent needs at least one official notice URL via --url or FRESHLOOP_HOLIDAY_NOTICE_URLS"
        );
    }

    let client = reqwest::Client::builder()
        .user_agent("FreshLoop holiday-agent/1.0")
        .build()?;

    let mut merged = load_calendar_from_path(&output_path)
        .ok()
        .or_else(|| bundled_calendar().ok())
        .unwrap_or_else(empty_calendar);

    for url in urls {
        let html = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("fetch holiday notice {}", url))?
            .error_for_status()
            .with_context(|| format!("holiday notice returned non-success status {}", url))?
            .text()
            .await
            .with_context(|| format!("read holiday notice body {}", url))?;
        let text = html_to_text(&html);
        let year = extract_notice_year(&text)
            .ok_or_else(|| anyhow::anyhow!("could not infer holiday notice year from {}", url))?;
        let source_title = extract_notice_title(&text, year);
        let year_entry = extract_year_from_notice(year, &source_title, &url, &text)?;
        upsert_year(&mut merged, year_entry);
    }

    merged.generated_at = chrono::Utc::now().to_rfc3339();
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, serde_json::to_string_pretty(&merged)?)?;
    println!("{}", output_path.display());
    Ok(())
}

fn print_holiday_agent_usage() {
    println!(
        "Usage: cortex holiday-agent --url <official-notice-url> [--url <url>...] [--output <path>]\n\
         Output defaults to ~/.freshloop/cache/holiday_calendar.json.\n\
         Multiple URLs are merged by year; later URLs replace the same year."
    );
}

fn bundled_calendar() -> Result<HolidayCalendarFile> {
    serde_json::from_str(BUNDLED_CALENDAR).context("parse bundled holiday calendar")
}

fn load_runtime_calendar() -> Result<HolidayCalendarFile> {
    #[cfg(test)]
    {
        let explicit_path = std::env::var("FRESHLOOP_HOLIDAY_CALENDAR_PATH")
            .context("FRESHLOOP_HOLIDAY_CALENDAR_PATH is not set in tests")?;
        return load_calendar_from_path(Path::new(&explicit_path));
    }

    #[cfg(not(test))]
    load_calendar_from_path(&runtime_calendar_path())
}

#[cfg(not(test))]
fn runtime_calendar_path() -> PathBuf {
    std::env::var("FRESHLOOP_HOLIDAY_CALENDAR_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_runtime_calendar_path())
}

fn default_runtime_calendar_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".freshloop/cache/holiday_calendar.json")
}

fn load_calendar_from_path(path: &Path) -> Result<HolidayCalendarFile> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

fn empty_calendar() -> HolidayCalendarFile {
    HolidayCalendarFile {
        schema_version: 1,
        generated_at: chrono::Utc::now().to_rfc3339(),
        years: Vec::new(),
    }
}

fn lookup_official_holiday(calendar: &HolidayCalendarFile, date: NaiveDate) -> Option<String> {
    calendar
        .years
        .iter()
        .find(|year| year.year == date.year())
        .and_then(|year| {
            year.holidays.iter().find_map(|holiday| {
                let start = parse_date(&holiday.start)?;
                let end = parse_date(&holiday.end)?;
                if (start..=end).contains(&date) {
                    Some(holiday.name.clone())
                } else {
                    None
                }
            })
        })
}

fn lookup_official_adjusted_workday(
    calendar: &HolidayCalendarFile,
    date: NaiveDate,
) -> Option<String> {
    calendar
        .years
        .iter()
        .find(|year| year.year == date.year())
        .and_then(|year| {
            year.adjusted_workdays.iter().find_map(|workday| {
                if parse_date(&workday.date) == Some(date) {
                    Some(workday.name.clone())
                } else {
                    None
                }
            })
        })
}

fn projected_holiday(date: NaiveDate) -> Option<String> {
    let year = date.year();
    let national_start = NaiveDate::from_ymd_opt(year, 10, 1)?;
    let mid_autumn = lunar_festival_date(year, 8, 15);
    if mid_autumn.is_some_and(|start| ranges_overlap(national_start, 7, start, 3)) {
        let mid_autumn = mid_autumn.unwrap();
        let start = national_start.min(mid_autumn);
        let end = national_start
            .checked_add_signed(Duration::days(7))
            .unwrap()
            .max(mid_autumn.checked_add_signed(Duration::days(2)).unwrap());
        if (start..=end).contains(&date) {
            return Some("国庆中秋假期".to_string());
        }
    }

    if date_in_range(date, national_start, 7)
        || mid_autumn.is_some_and(|start| date_in_range(date, start, 3))
    {
        if date_in_range(date, national_start, 7) {
            return Some("国庆假期".to_string());
        }
        if mid_autumn.is_some_and(|start| date_in_range(date, start, 3)) {
            return Some("中秋假期".to_string());
        }
    }

    if let Some(new_year) = lunar_festival_date(year, 1, 1) {
        let new_year_eve = new_year.checked_sub_signed(Duration::days(1))?;
        let start = projected_spring_festival_start(new_year_eve);
        if (start..=new_year.checked_add_signed(Duration::days(6))?).contains(&date) {
            return Some("春节假期".to_string());
        }
    }

    let mut solar_holidays = vec![
        ("元旦假期", NaiveDate::from_ymd_opt(year, 1, 1)?, 3),
        ("清明假期", projected_qingming_date(year)?, 3),
        ("劳动节假期", NaiveDate::from_ymd_opt(year, 5, 1)?, 5),
    ];
    if let Some(duanwu) = lunar_festival_date(year, 5, 5) {
        solar_holidays.push(("端午假期", duanwu, 3));
    }

    solar_holidays
        .into_iter()
        .find_map(|(name, start, days)| date_in_range(date, start, days).then(|| name.to_string()))
}

fn projected_spring_festival_start(new_year_eve: NaiveDate) -> NaiveDate {
    if new_year_eve.weekday() == Weekday::Sat {
        new_year_eve
    } else {
        new_year_eve - Duration::days(1)
    }
}

fn date_in_range(date: NaiveDate, start: NaiveDate, days: i64) -> bool {
    let end = start + Duration::days(days - 1);
    (start..=end).contains(&date)
}

fn ranges_overlap(
    left_start: NaiveDate,
    left_days: i64,
    right_start: NaiveDate,
    right_days: i64,
) -> bool {
    let left_end = left_start + Duration::days(left_days - 1);
    let right_end = right_start + Duration::days(right_days - 1);
    left_start <= right_end && right_start <= left_end
}

fn lunar_festival_date(year: i32, month: u8, day: u8) -> Option<NaiveDate> {
    let year = u16::try_from(year).ok()?;
    LunisolarDate::from_ymd(year, month, false, day)
        .ok()
        .map(Into::into)
}

fn projected_qingming_date(year: i32) -> Option<NaiveDate> {
    // A compact approximation is enough for a fallback label; official calendars override it.
    let day = if matches!(year, 2026 | 2027 | 2028) {
        4
    } else {
        5
    };
    NaiveDate::from_ymd_opt(year, 4, day)
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn html_to_text(html: &str) -> String {
    let without_scripts = Regex::new(r"(?is)<script.*?</script>")
        .unwrap()
        .replace_all(html, " ");
    let without_scripts = Regex::new(r"(?is)<style.*?</style>")
        .unwrap()
        .replace_all(&without_scripts, " ");
    let without_tags = Regex::new(r"(?is)<[^>]+>")
        .unwrap()
        .replace_all(&without_scripts, " ");
    let decoded = without_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    Regex::new(r"\s+")
        .unwrap()
        .replace_all(&decoded, " ")
        .trim()
        .to_string()
}

fn extract_notice_year(text: &str) -> Option<i32> {
    Regex::new(r"20\d{2}年部分节假日安排")
        .unwrap()
        .find(text)
        .and_then(|m| m.as_str().get(0..4))
        .and_then(|year| year.parse().ok())
}

fn extract_notice_title(text: &str, year: i32) -> String {
    let marker = format!("国务院办公厅关于{}年部分节假日安排的通知", year);
    if text.contains(&marker) {
        marker
    } else {
        format!("{}年部分节假日安排", year)
    }
}

fn extract_year_from_notice(
    year: i32,
    source_title: &str,
    source_url: &str,
    text: &str,
) -> Result<HolidayYear> {
    let mut holidays = Vec::new();
    let mut adjusted_workdays = Vec::new();
    let section_re =
        Regex::new(r"(?:^|[。；\s])([一二三四五六七八九十]+)、([^：:]{2,12})[：:]").unwrap();
    let range_re = Regex::new(
        r"(?P<sm>\d{1,2})月(?P<sd>\d{1,2})日(?:（[^）]*）)?至(?:(?P<em>\d{1,2})月)?(?P<ed>\d{1,2})日",
    )
    .unwrap();
    let date_re = Regex::new(r"(?P<m>\d{1,2})月(?P<d>\d{1,2})日").unwrap();

    let sections: Vec<_> = section_re.captures_iter(text).collect();
    for (index, captures) in sections.iter().enumerate() {
        let header = captures.get(0).unwrap();
        let body_start = header.end();
        let body_end = sections
            .get(index + 1)
            .and_then(|next| next.get(0))
            .map(|next| next.start())
            .unwrap_or(text.len());
        let raw_name = captures.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        let body = &text[body_start..body_end];
        let name = normalize_holiday_name(raw_name);
        if let Some(range) = range_re.captures(body) {
            let start_month = capture_u32(&range, "sm")?;
            let start_day = capture_u32(&range, "sd")?;
            let end_month = capture_u32(&range, "em").unwrap_or(start_month);
            let end_day = capture_u32(&range, "ed")?;
            let start = NaiveDate::from_ymd_opt(year, start_month, start_day)
                .ok_or_else(|| anyhow::anyhow!("invalid start date for {}", name))?;
            let end = NaiveDate::from_ymd_opt(year, end_month, end_day)
                .ok_or_else(|| anyhow::anyhow!("invalid end date for {}", name))?;
            holidays.push(HolidayPeriod {
                name: format!("{}假期", name),
                start: start.format("%Y-%m-%d").to_string(),
                end: end.format("%Y-%m-%d").to_string(),
            });
        }

        for sentence in body
            .split('。')
            .filter(|sentence| sentence.contains("上班"))
        {
            for workday in date_re.captures_iter(sentence) {
                let month = capture_u32(&workday, "m")?;
                let day = capture_u32(&workday, "d")?;
                let date = NaiveDate::from_ymd_opt(year, month, day)
                    .ok_or_else(|| anyhow::anyhow!("invalid adjusted workday for {}", name))?;
                adjusted_workdays.push(AdjustedWorkday {
                    date: date.format("%Y-%m-%d").to_string(),
                    name: format!("{}调休工作日", adjusted_workday_prefix(&name)),
                });
            }
        }
    }

    if holidays.is_empty() {
        anyhow::bail!("no holiday ranges extracted for {}", year);
    }

    adjusted_workdays.sort_by(|left, right| left.date.cmp(&right.date));
    holidays.sort_by(|left, right| left.start.cmp(&right.start));

    Ok(HolidayYear {
        year,
        source_title: source_title.to_string(),
        source_url: source_url.to_string(),
        holidays,
        adjusted_workdays,
    })
}

fn capture_u32(captures: &regex::Captures<'_>, name: &str) -> Result<u32> {
    captures
        .name(name)
        .ok_or_else(|| anyhow::anyhow!("missing capture {}", name))?
        .as_str()
        .parse::<u32>()
        .with_context(|| format!("parse capture {}", name))
}

fn normalize_holiday_name(name: &str) -> String {
    match name.trim() {
        combined if combined.contains("国庆") && combined.contains("中秋") => {
            "国庆中秋".to_string()
        }
        "清明节" => "清明".to_string(),
        "端午节" => "端午".to_string(),
        "中秋节" => "中秋".to_string(),
        "国庆节" => "国庆".to_string(),
        "元旦" => "元旦".to_string(),
        "春节" => "春节".to_string(),
        "劳动节" => "劳动节".to_string(),
        other => other.to_string(),
    }
}

fn adjusted_workday_prefix(name: &str) -> &str {
    match name {
        "劳动节" => "劳动",
        other => other,
    }
}

fn upsert_year(calendar: &mut HolidayCalendarFile, year_entry: HolidayYear) {
    calendar.years.retain(|entry| entry.year != year_entry.year);
    calendar.years.push(year_entry);
    calendar.years.sort_by_key(|entry| entry.year);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn bundled_calendar_knows_2026_official_holidays_and_workdays() {
        let spring = lookup_holiday(date(2026, 2, 17)).unwrap();
        assert_eq!(spring.name, "春节假期");
        assert_eq!(spring.certainty, HolidayCertainty::Official);

        assert_eq!(
            lookup_adjusted_workday(date(2026, 2, 14)).as_deref(),
            Some("春节调休工作日")
        );
    }

    #[test]
    fn projected_calendar_supports_future_spring_festival() {
        let spring = lookup_holiday(date(2027, 2, 5)).unwrap();
        assert_eq!(spring.name, "春节假期");
        assert_eq!(spring.certainty, HolidayCertainty::Projected);
    }

    #[test]
    fn projected_calendar_merges_2028_national_day_and_mid_autumn() {
        let holiday = lookup_holiday(date(2028, 10, 8)).unwrap();
        assert_eq!(holiday.name, "国庆中秋假期");
        assert_eq!(holiday.certainty, HolidayCertainty::Projected);
    }

    #[test]
    fn extracts_structured_calendar_from_official_notice_text() {
        let text = "国务院办公厅关于2026年部分节假日安排的通知。一、元旦：1月1日（周四）至3日（周六）放假调休，共3天。1月4日（周日）上班。二、春节：2月15日（农历腊月二十八、周日）至23日（农历正月初七、周一）放假调休，共9天。2月14日（周六）、2月28日（周六）上班。";
        let year = extract_year_from_notice(
            2026,
            "国务院办公厅关于2026年部分节假日安排的通知",
            "https://example.test",
            text,
        )
        .unwrap();

        assert_eq!(year.holidays[0].name, "元旦假期");
        assert_eq!(year.holidays[0].start, "2026-01-01");
        assert_eq!(year.holidays[0].end, "2026-01-03");
        assert_eq!(year.adjusted_workdays.len(), 3);
        assert_eq!(year.adjusted_workdays[1].date, "2026-02-14");
        assert_eq!(year.adjusted_workdays[1].name, "春节调休工作日");
    }
}
