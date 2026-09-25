//! Built-in skills, embedded in the binary: an ESP32 starter pack for C/C++
//! developers doing physical AI. Always available, no install needed.
//! A project/global skill with the same name shadows the built-in.

use std::path::PathBuf;

use crate::{parse_frontmatter_from, Skill, SkillSource};

macro_rules! builtin {
    ($name:literal) => {{
        const TEXT: &str = include_str!(concat!("../skills/", $name, "/SKILL.md"));
        TEXT
    }};
}

const BUILTINS: &[(&str, &str)] = &[
    ("esp32-start", builtin!("esp32-start")),
    ("esp-idf-patterns", builtin!("esp-idf-patterns")),
    ("esp32-gpio-adc-pwm", builtin!("esp32-gpio-adc-pwm")),
    ("esp32-buses", builtin!("esp32-buses")),
    ("esp32-wifi-mqtt", builtin!("esp32-wifi-mqtt")),
    ("esp32-power-sleep", builtin!("esp32-power-sleep")),
    ("esp32-ota-debug", builtin!("esp32-ota-debug")),
    ("esp32-edge-ai", builtin!("esp32-edge-ai")),
    ("esp32-safety", builtin!("esp32-safety")),
];

/// All embedded skills (frontmatter parsed at call time; cheap).
pub fn builtin_skills() -> Vec<Skill> {
    BUILTINS
        .iter()
        .filter_map(|(fallback, text)| {
            let (name, description) = parse_frontmatter_from(text)
                .unwrap_or_else(|| (fallback.to_string(), String::new()));
            Some(Skill {
                name,
                description,
                dir: PathBuf::new(),
                source: SkillSource::Builtin,
                embedded: Some(text),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_body, render_catalog};

    #[test]
    fn all_builtins_have_name_description_and_body() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 9);
        for s in &skills {
            assert!(!s.name.is_empty(), "builtin missing name");
            assert!(!s.description.is_empty(), "builtin {} missing description", s.name);
            let body = load_body(s).unwrap();
            assert!(body.len() > 500, "builtin {} body too thin", s.name);
            assert!(!body.contains("description:"), "builtin {} leaks frontmatter", s.name);
        }
        let catalog = render_catalog(&skills);
        assert!(catalog.contains("[builtin]"));
        assert!(catalog.contains("esp32-edge-ai"));
    }
}
