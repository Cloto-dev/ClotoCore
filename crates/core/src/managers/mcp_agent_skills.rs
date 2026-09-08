//! Agent skills — the operator's own procedures, indexed always and loaded on
//! demand.
//!
//! An agent's instruction files (`CLAUDE.md`, `AGENTS.md`, `MEMORY.md`) ride on
//! every dispatch, so they are bounded to what is worth paying for every turn.
//! A procedure the agent needs twice a month does not belong there. Skills are
//! the other half: the kernel lists what exists on every dispatch — an id and
//! one line of description, which is what choosing requires — and the body
//! arrives only when the agent asks for it by calling `mgp.skill.load`.
//!
//! The layout mirrors the instruction directory that is already kernel-owned:
//!
//! ```text
//! <data_dir>/agents/<agent_id>/skills/<skill_id>/SKILL.md
//! ```
//!
//! The directory name is the skill's identity. The frontmatter may carry a
//! `name:` for display, but the id the agent loads by is the directory, because
//! that is the name the filesystem guarantees is unique and path-safe.

use super::mcp::{clamp_instructions, is_safe_path_segment};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Directory under an agent's instruction directory holding its skills.
const SKILLS_DIR: &str = "skills";

/// The one file the kernel reads inside a skill directory.
const SKILL_FILE: &str = "SKILL.md";

/// Ceiling on one description in the always-loaded index.
///
/// An index line is a decision aid, not the content: it exists so the agent can
/// tell whether loading the skill is worth a turn. A description that needs
/// more than this is describing instead of naming, and the body is where that
/// belongs.
const SKILL_DESCRIPTION_CHARS: usize = 200;

/// Ceiling on the whole index block.
///
/// The index rides on every dispatch, so it gets one source's share of the
/// prompt — the same allowance a single instruction file gets, for the same
/// reason.
const SKILL_INDEX_TOTAL_CHARS: usize = 3_000;

/// Ceiling on a loaded skill body.
///
/// Larger than a per-file instruction allowance because a skill is loaded
/// deliberately and is the point of the call; bounded anyway because the loaded
/// body stays in the transcript and is re-sent on every following turn of the
/// conversation.
const SKILL_BODY_CHARS: usize = 8_000;

/// A skill the operator placed for an agent.
pub(super) struct AgentSkill {
    /// Directory name — the id `mgp.skill.load` takes. Path-safe by
    /// construction: a directory whose name is not a safe path segment is not
    /// listed.
    pub id: String,
    /// `description:` from the SKILL.md frontmatter. Empty when the file has
    /// none, which is listed as such rather than hidden — an operator who wrote
    /// a skill and no description should see the skill, not lose it.
    pub description: String,
}

/// Path of `agent_id`'s skills directory under `base`, or `None` when the id
/// cannot be used as a path segment.
fn skills_dir(base: &Path, agent_id: &str) -> Option<PathBuf> {
    if !is_safe_path_segment(agent_id) {
        warn!(
            agent_id,
            "agent id is not usable as a path segment; no skills were read"
        );
        return None;
    }
    Some(base.join(agent_id).join(SKILLS_DIR))
}

/// Split a leading YAML frontmatter block off `text`.
///
/// Returns `(frontmatter, body)`. Deliberately a line scan rather than a YAML
/// parse: the kernel reads exactly two scalar keys out of this block, and a
/// parser that accepts anchors, aliases and merge keys would be a larger
/// surface than the feature.
fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let rest = text.strip_prefix("---\n").or_else(|| {
        text.strip_prefix("---\r\n")
            .or_else(|| text.strip_prefix("---").filter(|r| r.is_empty()))
    });
    let Some(rest) = rest else {
        return (None, text);
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let front = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (Some(front), body);
        }
        offset += line.len();
    }
    // An opening fence with no closing one is not frontmatter; treating the
    // rest of the file as metadata would silently swallow the whole skill.
    (None, text)
}

/// Read one `key: value` out of a frontmatter block.
fn frontmatter_value(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim().trim_matches(['"', '\''].as_ref()).trim();
        if v.is_empty() {
            return None;
        }
        return Some(v.to_string());
    }
    None
}

/// List the skills `agent_id` has under `base`, sorted by id.
///
/// Sorted for the reason the instruction file list is a fixed array: the order
/// a filesystem hands back its entries is not stable, and an order that moves
/// between dispatches defeats prompt caching and makes two dispatches
/// impossible to diff.
///
/// Takes the base directory rather than reading
/// [`super::mcp::agent_instructions_root`] itself, so the listing can be
/// tested against a temporary tree.
pub(super) async fn list_skills_in(base: &Path, agent_id: &str) -> Vec<AgentSkill> {
    let Some(dir) = skills_dir(base, agent_id) else {
        return Vec::new();
    };
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return Vec::new();
    };

    let mut skills: Vec<AgentSkill> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(id) = entry.file_name().into_string() else {
            continue;
        };
        // A directory whose name cannot be a path segment could not be loaded
        // by id afterwards, so listing it would advertise something the loader
        // must refuse.
        if !is_safe_path_segment(&id) {
            continue;
        }
        let Ok(text) = tokio::fs::read_to_string(entry.path().join(SKILL_FILE)).await else {
            continue;
        };
        let (front, body) = split_frontmatter(&text);
        // A directory with an empty SKILL.md holds no procedure; loading it
        // would return nothing, so it is not offered.
        if body.trim().is_empty() {
            continue;
        }
        let description = front
            .and_then(|f| frontmatter_value(f, "description"))
            .unwrap_or_default();
        skills.push(AgentSkill { id, description });
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    skills
}

/// Compose the always-loaded index for `skills`, or `None` when there are none.
pub(super) fn compose_skill_index(skills: &[AgentSkill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for skill in skills {
        let description = if skill.description.is_empty() {
            "(no description — load it to see what it does)".to_string()
        } else {
            clamp_instructions(&skill.description, SKILL_DESCRIPTION_CHARS)
        };
        let line = format!("- {}: {}", skill.id, description);
        let cost = line.chars().count();
        if used + cost > SKILL_INDEX_TOTAL_CHARS {
            omitted += 1;
            continue;
        }
        used += cost;
        lines.push(line);
    }

    // Every skill overflowed the budget, which is a broken index rather than a
    // short one: it would name nothing while claiming skills exist.
    if lines.is_empty() {
        warn!(
            count = skills.len(),
            "no skill fit the index budget; the index was left out"
        );
        return None;
    }

    let mut block = String::from(
        "# Skills\n\n\
         Your operator placed these skills in this agent's skill directory. \
         Each line is an id and what that skill is for — the procedure itself \
         is not here. When one of them covers the task at hand, call \
         `mgp.skill.load` with its id and follow what it returns; do not act on \
         the description alone.\n\n",
    );
    for line in lines {
        block.push_str(&line);
        block.push('\n');
    }
    if omitted > 0 {
        use std::fmt::Write as _;
        let plural = if omitted == 1 { "" } else { "s" };
        let _ = write!(
            block,
            "\n({omitted} further skill{plural} did not fit the prompt budget \
             and {} left out; ask your operator if you need {}.)\n",
            if omitted == 1 { "was" } else { "were" },
            if omitted == 1 { "it" } else { "them" },
        );
    }
    Some(block)
}

/// Read the body of one skill, or `None` when `skill_id` names no readable
/// skill under `base`.
///
/// The frontmatter is stripped: it is metadata the index already carried, and
/// the agent asked for the procedure.
pub(super) async fn read_skill_body_in(
    base: &Path,
    agent_id: &str,
    skill_id: &str,
) -> Option<String> {
    // The skill id becomes a directory name, so an id that could climb out of
    // the agent's own directory must never reach the filesystem. Unlike the
    // agent id, this one is chosen by the model, which is exactly why the check
    // is here and not left to the caller.
    if !is_safe_path_segment(skill_id) {
        warn!(
            agent_id,
            skill_id, "skill id is not usable as a path segment; nothing was read"
        );
        return None;
    }
    let dir = skills_dir(base, agent_id)?;
    let text = tokio::fs::read_to_string(dir.join(skill_id).join(SKILL_FILE))
        .await
        .ok()?;
    let (_, body) = split_frontmatter(&text);
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    Some(clamp_instructions(body, SKILL_BODY_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Write `SKILL.md` for `skill_id` under a temporary agents root.
    fn write_skill(base: &Path, agent_id: &str, skill_id: &str, text: &str) {
        let dir = base.join(agent_id).join(SKILLS_DIR).join(skill_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SKILL_FILE), text).unwrap();
    }

    fn skill_text(description: &str, body: &str) -> String {
        format!("---\nname: display\ndescription: {description}\n---\n\n{body}\n")
    }

    #[tokio::test]
    async fn skills_are_listed_in_id_order_whatever_the_filesystem_says() {
        let base = tempfile::tempdir().unwrap();
        // Created in an order that is neither alphabetical nor reverse, so a
        // listing that merely echoed `read_dir` would have to be lucky twice.
        for id in ["deploy", "audit", "release"] {
            write_skill(base.path(), "agent.a", id, &skill_text(id, "body"));
        }

        let skills = list_skills_in(base.path(), "agent.a").await;

        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["audit", "deploy", "release"]);
    }

    #[tokio::test]
    async fn a_skill_directory_without_a_body_is_not_offered() {
        let base = tempfile::tempdir().unwrap();
        write_skill(
            base.path(),
            "agent.a",
            "real",
            &skill_text("does a thing", "body"),
        );
        // Frontmatter only: nothing to load, so nothing to advertise.
        write_skill(
            base.path(),
            "agent.a",
            "hollow",
            "---\ndescription: promises much\n---\n\n   \n",
        );
        std::fs::create_dir_all(base.path().join("agent.a").join(SKILLS_DIR).join("empty"))
            .unwrap();

        let ids: Vec<String> = list_skills_in(base.path(), "agent.a")
            .await
            .into_iter()
            .map(|s| s.id)
            .collect();

        assert_eq!(ids, vec!["real".to_string()]);
    }

    #[tokio::test]
    async fn a_skill_without_a_description_is_listed_as_having_none() {
        let base = tempfile::tempdir().unwrap();
        write_skill(base.path(), "agent.a", "bare", "no frontmatter at all\n");

        let skills = list_skills_in(base.path(), "agent.a").await;
        assert_eq!(skills.len(), 1);
        assert!(skills[0].description.is_empty());

        let index = compose_skill_index(&skills).unwrap();
        assert!(index.contains("- bare: (no description"), "{index}");
    }

    #[tokio::test]
    async fn an_oversized_description_is_clamped_in_the_index() {
        let base = tempfile::tempdir().unwrap();
        let flood = "あ".repeat(SKILL_DESCRIPTION_CHARS + 500);
        write_skill(
            base.path(),
            "agent.a",
            "verbose",
            &skill_text(&flood, "body"),
        );

        let index = compose_skill_index(&list_skills_in(base.path(), "agent.a").await).unwrap();

        assert_eq!(
            index.matches('あ').count(),
            SKILL_DESCRIPTION_CHARS,
            "the description should be clamped to its ceiling"
        );
        assert!(index.contains("truncated by the kernel"), "{index}");
    }

    #[tokio::test]
    async fn skills_beyond_the_index_budget_are_counted_and_named_as_left_out() {
        let base = tempfile::tempdir().unwrap();
        // Each line costs about the per-description ceiling, so a handful of
        // them overflow an index budget measured in thousands.
        let long = "x".repeat(SKILL_DESCRIPTION_CHARS);
        for i in 0..40 {
            write_skill(
                base.path(),
                "agent.a",
                &format!("skill-{i:02}"),
                &skill_text(&long, "body"),
            );
        }

        let index = compose_skill_index(&list_skills_in(base.path(), "agent.a").await).unwrap();

        assert!(
            index.contains("further skill") && index.contains("left out"),
            "the index must say how many it could not fit: {index}"
        );
        assert!(
            index.chars().count() < SKILL_INDEX_TOTAL_CHARS + 1_000,
            "the index should stay near its budget, was {}",
            index.chars().count()
        );
    }

    #[tokio::test]
    async fn loading_returns_the_body_without_its_frontmatter() {
        let base = tempfile::tempdir().unwrap();
        write_skill(
            base.path(),
            "agent.a",
            "deploy",
            &skill_text("ships the thing", "## Steps\n\n1. build\n2. ship"),
        );

        let body = read_skill_body_in(base.path(), "agent.a", "deploy")
            .await
            .unwrap();

        assert!(body.starts_with("## Steps"), "{body}");
        assert!(!body.contains("description:"), "{body}");
        assert!(body.contains("2. ship"), "{body}");
    }

    #[tokio::test]
    async fn an_oversized_body_is_clamped_and_says_so() {
        let base = tempfile::tempdir().unwrap();
        let flood = "あ".repeat(SKILL_BODY_CHARS + 500);
        write_skill(base.path(), "agent.a", "long", &skill_text("big", &flood));

        let body = read_skill_body_in(base.path(), "agent.a", "long")
            .await
            .unwrap();

        assert_eq!(body.matches('あ').count(), SKILL_BODY_CHARS);
        assert!(body.contains("truncated by the kernel"), "{body}");
    }

    #[tokio::test]
    async fn an_unclosed_frontmatter_fence_does_not_swallow_the_body() {
        let base = tempfile::tempdir().unwrap();
        write_skill(
            base.path(),
            "agent.a",
            "broken",
            "---\ndescription: never closed\n\nthe actual procedure\n",
        );

        let body = read_skill_body_in(base.path(), "agent.a", "broken")
            .await
            .unwrap();

        assert!(
            body.contains("the actual procedure"),
            "a malformed fence must not hide the skill: {body}"
        );
    }

    #[tokio::test]
    async fn a_skill_id_that_climbs_out_of_the_directory_reads_nothing() {
        let base = tempfile::tempdir().unwrap();
        // A readable skill on the other side of the boundary: the assertion
        // below fails if the guard goes away, rather than passing because
        // there was nothing there to find.
        write_skill(
            base.path(),
            "agent.b",
            "secret",
            &skill_text("d", "the other agent's"),
        );
        write_skill(base.path(), "agent.a", "own", &skill_text("d", "mine"));

        for hostile in ["../../agent.b/skills/secret", "..", "a/b", "a\0b"] {
            assert!(
                read_skill_body_in(base.path(), "agent.a", hostile)
                    .await
                    .is_none(),
                "skill id {hostile:?} should read nothing"
            );
        }

        // Same bytes, reachable under a well-formed id.
        assert!(read_skill_body_in(base.path(), "agent.b", "secret")
            .await
            .unwrap()
            .contains("the other agent's"));
    }

    #[tokio::test]
    async fn an_agent_id_that_is_not_a_path_segment_lists_and_loads_nothing() {
        let base = tempfile::tempdir().unwrap();
        write_skill(base.path(), "agent.a", "own", &skill_text("d", "mine"));

        assert!(list_skills_in(base.path(), "../agent.a").await.is_empty());
        assert!(read_skill_body_in(base.path(), "../agent.a", "own")
            .await
            .is_none());
    }

    #[test]
    fn an_index_of_nothing_is_no_index() {
        assert!(compose_skill_index(&[]).is_none());
    }
}
