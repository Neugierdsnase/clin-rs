use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub links: Option<Vec<String>>,
    #[serde(default)]
    pub original_ext: Option<String>,
}

/// Extract the raw YAML text between a note's `---` frontmatter
/// delimiters, along with the remaining body. Returns `None` when
/// `content` doesn't open with a `---` delimiter, or has no closing one.
pub fn extract_raw(content: &str) -> Option<(&str, &str)> {
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return None;
    }

    let end_marker = "\n---";
    let end_idx = content[3..].find(end_marker)?;
    let frontmatter_str = &content[3..3 + end_idx];

    let remaining_start = 3 + end_idx + end_marker.len();
    let mut content_start = remaining_start;
    if content[remaining_start..].starts_with("\r\n") {
        content_start += 2;
    } else if content[remaining_start..].starts_with('\n') {
        content_start += 1;
    }

    Some((frontmatter_str, &content[content_start..]))
}

pub fn parse(content: &str) -> (Frontmatter, &str) {
    let Some((frontmatter_str, remaining_content)) = extract_raw(content) else {
        return (Frontmatter::default(), content);
    };

    match serde_yaml_ng::from_str::<Frontmatter>(frontmatter_str) {
        Ok(frontmatter) => (frontmatter, remaining_content),
        Err(_) => (Frontmatter::default(), content),
    }
}

pub fn serialize(frontmatter: &Frontmatter, content: &str) -> String {
    if frontmatter.tags.is_empty()
        && !frontmatter.pinned
        && frontmatter.title.is_none()
        && frontmatter.updated_at.is_none()
        && frontmatter.links.is_none()
        && frontmatter.original_ext.is_none()
    {
        return content.to_string();
    }

    match serde_yaml_ng::to_string(frontmatter) {
        Ok(yaml) => {
            let yaml = yaml.trim();
            let yaml = yaml.to_string();

            format!("---\n{yaml}\n---\n{content}")
        }
        Err(_) => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "Just some text";
        let (fm, remaining) = parse(content);
        assert!(fm.tags.is_empty());
        assert_eq!(remaining, "Just some text");
    }

    #[test]
    fn test_parse_with_frontmatter() {
        let content = "---\ntags:\n  - work\n  - urgent\n---\nHere is the content.";
        let (fm, remaining) = parse(content);
        assert_eq!(fm.tags, vec!["work", "urgent"]);
        assert_eq!(remaining, "Here is the content.");
    }

    #[test]
    fn test_serialize() {
        let fm = Frontmatter {
            tags: vec!["work".to_string()],
            pinned: false,
            ..Default::default()
        };
        let content = "My note";
        let serialized = serialize(&fm, content);
        assert!(serialized.starts_with("---\n"));
        assert!(serialized.contains("work"));
        assert!(serialized.ends_with("\n---\nMy note"));
    }
}
