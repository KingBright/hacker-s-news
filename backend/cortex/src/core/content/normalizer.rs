use regex::Regex;

pub fn clean_text_for_processing(input: &str, max_chars: usize) -> String {
    let re = Regex::new(r"<[^>]*>").expect("valid html stripping regex");
    let no_html = re.replace_all(input, " ");

    let entity_fixed = no_html
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'");

    let punct_fixed = entity_fixed
        .replace(",", "，")
        .replace("?", "？")
        .replace("!", "！")
        .replace("(", "（")
        .replace(")", "）");

    let noise_fixed = punct_fixed
        .replace("【", " ")
        .replace("】", " ")
        .replace("[", " ")
        .replace("]", " ")
        .replace("|", " ");

    let re_space = Regex::new(r"\s+").expect("valid whitespace regex");
    let clean = re_space.replace_all(&noise_fixed, " ");

    if clean.chars().count() > max_chars {
        let mut s: String = clean.chars().take(max_chars).collect();
        s.push_str("...");
        s
    } else {
        clean.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_html_entities_noise_and_whitespace() {
        let cleaned = clean_text_for_processing(
            "<p>【AI】Tom &amp; Jerry, really?</p>\n<span>Yes!</span>",
            100,
        );

        assert_eq!(cleaned, " AI Tom & Jerry， really？ Yes！ ");
    }

    #[test]
    fn truncates_by_chars() {
        let cleaned = clean_text_for_processing("abcdef", 3);
        assert_eq!(cleaned, "abc...");
    }
}
