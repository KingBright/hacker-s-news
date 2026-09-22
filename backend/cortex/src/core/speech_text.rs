//! Conservative spoken-only normalization. Never infer a name's pronunciation or
//! replace a polyphonic character globally; the external editor supplies context.
use regex::{Captures, Regex};
use std::sync::LazyLock;

static PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(-?\d+(?:\.\d+)?)\s*[%％]").unwrap());
static UNITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(\d+(?:\.\d+)?)\s*(GHz|MHz|kHz|km/h|ms|kg|℃|°C|°F)(?-u:\b)|(?P<temp>\d+(?:\.\d+)?)\s*℃",
    )
    .unwrap()
});
static TERMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(CPU|GPU|USB|HTML|HTTP|HTTPS)(?-u:\b)").unwrap());

pub fn prepare_speech(text: &str) -> String {
    let text = text.replace('〇', "零");
    let clean = text.replace(['\0', '\u{200b}', '\u{feff}'], "");
    let clean = PERCENT.replace_all(&clean, |c: &Captures| match c[1].strip_prefix('-') {
        Some(n) => format!("负百分之{n}"),
        None => format!("百分之{}", &c[1]),
    });
    let clean = UNITS.replace_all(&clean, |c: &Captures| {
        if let Some(t) = c.name("temp") {
            return format!("{}摄氏度", t.as_str());
        }
        let unit = match &c[2] {
            "GHz" => "吉赫兹",
            "MHz" => "兆赫兹",
            "kHz" => "千赫兹",
            "km/h" => "公里每小时",
            "ms" => "毫秒",
            "kg" => "千克",
            "℃" | "°C" => "摄氏度",
            "°F" => "华氏度",
            _ => unreachable!(),
        };
        format!("{}{unit}", &c[1])
    });
    TERMS
        .replace_all(&clean, |c: &Captures| match &c[1] {
            "CPU" => "中央处理器",
            "GPU" => "图形处理器",
            "USB" => "通用串行总线",
            "HTML" => "超文本标记语言",
            "HTTP" => "超文本传输协议",
            "HTTPS" => "安全超文本传输协议",
            _ => unreachable!(),
        })
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_zero_in_dates_is_spoken_as_zero() {
        assert_eq!(
            prepare_speech("二〇二六年九月二十二日"),
            "二零二六年九月二十二日"
        );
    }

    #[test]
    fn speaks_units_symbols_and_acronyms() {
        assert_eq!(
            prepare_speech("GPU 3.5GHz，增幅12.5%，-2%，温度25℃，延迟10ms。"),
            "图形处理器 3.5吉赫兹，增幅百分之12.5，负百分之2，温度25摄氏度，延迟10毫秒。"
        );
    }
    #[test]
    fn preserves_context_and_unknown_names() {
        let text="原料药 API、AI 文件、LLM 法学硕士。重庆银行行长强调：重新加载与重载卡车不同。OpenAI、DeepSeek、C++、AT&T、10msec。";
        assert_eq!(prepare_speech(text), text);
        assert_eq!(
            prepare_speech(&prepare_speech("GPU 20%")),
            prepare_speech("GPU 20%")
        );
    }
}
