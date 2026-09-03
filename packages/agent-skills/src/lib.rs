use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

pub fn load_skills(root: &Path) -> Result<Vec<Skill>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut skills = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let (name, description, instructions) =
            parse_skill(&raw, &entry.file_name().to_string_lossy())?;
        skills.push(Skill {
            name,
            description,
            instructions,
        });
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(skills)
}

pub fn select_skills<'a>(skills: &'a [Skill], request: &str, limit: usize) -> Vec<&'a Skill> {
    let request_words = words(request);
    let mut scored = skills
        .iter()
        .filter_map(|skill| {
            let candidate_words = words(&format!("{} {}", skill.name, skill.description));
            let score = request_words
                .iter()
                .filter(|word| candidate_words.contains(*word))
                .count();
            (score > 0).then_some((score, skill))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.name.cmp(&right.name))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, skill)| skill)
        .collect()
}

fn parse_skill(raw: &str, fallback_name: &str) -> Result<(String, String, String), String> {
    if !raw.starts_with("---") {
        return Ok((fallback_name.to_string(), String::new(), raw.to_string()));
    }
    let remainder = raw.trim_start_matches("---").trim_start();
    let (frontmatter, instructions) = remainder
        .split_once("---")
        .ok_or_else(|| "skill frontmatter is not closed".to_string())?;
    let field = |name: &str| {
        frontmatter.lines().find_map(|line| {
            line.split_once(':').and_then(|(key, value)| {
                (key.trim() == name).then(|| value.trim().trim_matches('"').to_string())
            })
        })
    };
    Ok((
        field("name").unwrap_or_else(|| fallback_name.to_string()),
        field("description").unwrap_or_default(),
        instructions.trim().to_string(),
    ))
}

fn words(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_skill_by_name_and_description_words() {
        let skills = vec![Skill {
            name: "rust-control-plane".to_string(),
            description: "Debug Rust scheduler jobs".to_string(),
            instructions: "Run focused tests".to_string(),
        }];
        assert_eq!(
            select_skills(&skills, "debug the Rust scheduler", 3).len(),
            1
        );
        assert!(select_skills(&skills, "translate French", 3).is_empty());
    }
}
