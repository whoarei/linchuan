use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Words {
    moods: Vec<String>,
    animals: Vec<String>,
}

static WORDS: LazyLock<Words> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../shared/device-names.json"))
        .expect("valid bundled device name dictionary")
});

/// A label derived from a random device ID, independent of its authentication.
pub(crate) fn for_id(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let seed = u64::from_le_bytes(digest[..8].try_into().unwrap());
    let index = (seed % (WORDS.moods.len() * WORDS.animals.len()) as u64) as usize;
    format!(
        "{}{}",
        WORDS.moods[index / WORDS.animals.len()],
        WORDS.animals[index % WORDS.animals.len()]
    )
}

pub(crate) fn browser_platform(user_agent: &str) -> String {
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
    [os, browser]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_is_unique_and_names_are_stable_valid_labels() {
        let mut names = std::collections::HashSet::new();
        for mood in &WORDS.moods {
            for animal in &WORDS.animals {
                let name = format!("{mood}{animal}");
                assert_eq!(crate::clean_name(&name).unwrap(), name);
                assert!(names.insert(name));
            }
        }
        assert!(names.len() >= 2000);
        assert!(names.contains(&for_id("device-one")));
        assert_eq!(for_id("device-one"), "自在小鹿");
    }

    #[test]
    fn browser_platforms_recognize_overlapping_user_agents() {
        for (ua, expected) in [
            ("Windows Chrome/131 Safari/537 Edg/131", "Windows · Edge"),
            ("Linux Android Chrome/131 Safari/537", "Android · Chrome"),
            ("iPhone Mac OS X CriOS/131 Safari/604", "iOS · Chrome"),
            ("Macintosh Safari/605", "Mac · Safari"),
            ("Linux Firefox/132", "Linux · Firefox"),
            ("", "浏览器"),
        ] {
            assert_eq!(browser_platform(ua), expected);
        }
    }
}
