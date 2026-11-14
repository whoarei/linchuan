/// Names are labels only. Device IDs and pairing credentials remain independent.
pub(crate) fn native(id: &str) -> String {
    let hostname = hostname::get().ok();
    native_with_hostname(
        crate::platform(),
        hostname.as_deref().and_then(|h| h.to_str()),
        id,
    )
}

fn native_with_hostname(platform: &str, hostname: Option<&str>, id: &str) -> String {
    let name: String = hostname
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(64)
        .collect();
    if !name.is_empty()
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "localhost"
                | "localhost.localdomain"
                | "localhost6"
                | "localhost6.localdomain6"
                | "(none)"
                | "android"
        )
    {
        name
    } else {
        format!("{platform} · {}", suffix(id))
    }
}

pub(crate) fn browser(user_agent: &str, id: &str) -> String {
    let os = if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("iPhone")
        || user_agent.contains("iPad")
        || user_agent.contains("iPod")
    {
        "iOS"
    } else if user_agent.contains("Windows") {
        "Windows"
    } else if user_agent.contains("CrOS") {
        "ChromeOS"
    } else if user_agent.contains("Macintosh") || user_agent.contains("Mac OS X") {
        "Mac"
    } else if user_agent.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    let browser = if user_agent.contains("Edg/")
        || user_agent.contains("EdgA/")
        || user_agent.contains("EdgiOS/")
    {
        "Edge"
    } else if user_agent.contains("OPR/") || user_agent.contains("Opera") {
        "Opera"
    } else if user_agent.contains("SamsungBrowser/") {
        "三星浏览器"
    } else if user_agent.contains("Firefox/") || user_agent.contains("FxiOS/") {
        "Firefox"
    } else if user_agent.contains("Chrome/") || user_agent.contains("CriOS/") {
        "Chrome"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "浏览器"
    };
    [os.to_string(), browser.to_string(), suffix(id)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn suffix(id: &str) -> String {
    id.chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_names_prefer_clean_hostname_and_fall_back_for_generic_hosts() {
        assert_eq!(
            native_with_hostname("Windows", Some(" 小林的电脑\n"), "a7f2"),
            "小林的电脑"
        );
        assert_eq!(
            native_with_hostname("Android", Some("localhost"), "a7f2"),
            "Android · A7F2"
        );
        assert_eq!(native_with_hostname("Linux", None, "a7f2"), "Linux · A7F2");
        assert_eq!(
            native_with_hostname("Linux", Some(&"名".repeat(80)), "a7f2")
                .chars()
                .count(),
            64
        );
    }

    #[test]
    fn browser_names_recognize_overlapping_user_agents() {
        for (ua, expected) in [
            (
                "Windows Chrome/131 Safari/537 Edg/131",
                "Windows · Edge · A7F2",
            ),
            (
                "Linux Android Chrome/131 Safari/537",
                "Android · Chrome · A7F2",
            ),
            (
                "iPhone Mac OS X CriOS/131 Safari/604",
                "iOS · Chrome · A7F2",
            ),
            ("Macintosh Safari/605", "Mac · Safari · A7F2"),
            ("Linux Firefox/132", "Linux · Firefox · A7F2"),
            ("", "浏览器 · A7F2"),
        ] {
            assert_eq!(browser(ua, "a7f2"), expected);
        }
    }
}
