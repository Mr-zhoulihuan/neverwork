//! 技能/插件的真实安装：落盘为 `skills/<id>/SKILL.md`。
//!
//! 三条约定：
//! 1. **文本技能安装 = 写文件**，卸载 = 删目录。文件正文就是发给模型的正文，
//!    用户在资源管理器里能看见、能改、能备份。
//! 2. **MCP 技能不走这里**：它们是 `settings.mcp_servers` 里的进程配置，
//!    由 `neverwork_agent::mcp` 按需拉起（本模块不执行任何外部进程）。
//! 3. **id 来自目录或导入文件的 front-matter，必须消毒**：只留
//!    `[a-z0-9._-]`，且不允许出现 `..`，防止拼路径逃出 skills 目录。

use std::path::{Path, PathBuf};

use neverwork_core::{Skill, SkillCategory, SkillKind};

/// 技能安装根目录（`<config_dir>/skills`）。
pub fn skills_dir() -> PathBuf {
    neverwork_core::config::config_dir().join("skills")
}

/// id 消毒成单层目录名：小写、只留 `[a-z0-9-_.]`，连续点折成 `_`，
/// 首尾的 `.`/`_` 去掉；空则回落 `skill`。
fn dir_name(id: &str) -> String {
    let mut out = String::new();
    let mut last_dot = false;
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c.to_ascii_lowercase());
            last_dot = false;
        } else if c == '.' && !last_dot && !out.is_empty() {
            out.push('.');
            last_dot = true;
        } else if !out.ends_with('_') {
            out.push('_');
            last_dot = false;
        }
    }
    let out = out.trim_matches(|c| c == '.' || c == '_').to_string();
    if out.is_empty() {
        "skill".into()
    } else {
        out
    }
}

pub fn skill_dir(id: &str) -> PathBuf {
    skills_dir().join(dir_name(id))
}

/// 某个技能的落盘文件。
pub fn skill_file(id: &str) -> PathBuf {
    skill_dir(id).join("SKILL.md")
}

/// 磁盘上是否已安装（以文件存在为准，不信任内存标记）。
pub fn is_installed(id: &str) -> bool {
    skill_file(id).is_file()
}

/// 安装（或覆盖）文本技能，返回写入的文件路径。
pub fn install_text(skill: &Skill) -> Result<PathBuf, String> {
    install_text_in(&skills_dir(), skill)
}

fn install_text_in(root: &Path, skill: &Skill) -> Result<PathBuf, String> {
    let file = root.join(dir_name(&skill.id)).join("SKILL.md");
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建技能目录失败：{e}"))?;
    }
    let body = if skill.detail.trim().is_empty() {
        format!("# {}\n\n{}", skill.name.trim(), skill.summary.trim())
    } else {
        skill.detail.trim().to_string()
    };
    let text = format!(
        "---\nid: {}\nname: {}\nsummary: {}\ncategory: {}\n---\n\n{}\n",
        skill.id.trim(),
        skill.name.trim(),
        skill.summary.trim(),
        skill.category.key(),
        body
    );
    std::fs::write(&file, text).map_err(|e| format!("写入 SKILL.md 失败：{e}"))?;
    Ok(file)
}

/// 卸载：删掉整个技能目录。目录本来就不存在算成功。
pub fn uninstall_text(id: &str) -> Result<(), String> {
    let dir = skill_dir(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除技能目录失败：{e}"))?;
    }
    Ok(())
}

/// 读取已安装技能的正文（去掉 front-matter）。未安装/读失败返回 `None`。
pub fn read_body(id: &str) -> Option<String> {
    let text = std::fs::read_to_string(skill_file(id)).ok()?;
    let (_, body) = split_front_matter(&text);
    let body = body.trim().to_string();
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// 从 `.md` 文本解析成 Skill（导入用）。
///
/// front-matter 可带 `id` / `name` / `summary` / `category`，缺省值：
/// id 由文件名生成（`local.<slug>`），名称取正文第一个非空行（去掉 `#`），
/// 简介取第二行非标题文本，分类回落开发编程。
pub fn parse_md(text: &str, file_stem: &str) -> Skill {
    let (meta, body) = split_front_matter(text);
    let get = |k: &str| {
        meta.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    let first_line = body
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let name = get("name").unwrap_or_else(|| {
        let t = first_line.trim_start_matches('#').trim();
        if t.is_empty() {
            file_stem.to_string()
        } else {
            t.chars().take(24).collect()
        }
    });
    let id = get("id").unwrap_or_else(|| format!("local.{}", slug(file_stem)));
    let summary = get("summary")
        // Claude 生态的 SKILL.md 用 `description` 写简介
        .or_else(|| get("description"))
        .unwrap_or_else(|| {
            body.lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.chars().take(60).collect::<String>())
                .unwrap_or_else(|| format!("从本地文件导入：{file_stem}"))
        });
    let category = get("category")
        .map(|c| SkillCategory::from_key(&c))
        .unwrap_or(SkillCategory::Dev);
    let glyph = name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "技".into());

    Skill {
        id,
        name,
        summary,
        detail: body.trim().to_string(),
        category,
        author: "local".into(),
        installs: 0,
        stars: 0,
        glyph,
        installed: true,
        kind: SkillKind::Text,
        mcp: None,
    }
}

/// 拆 front-matter：返回 (键值对, 正文)。没有 front-matter 时键值对为空。
///
/// 兼容 YAML 块标量：`description: >` / `|` 后面缩进的几行会拼成一段，
/// 否则 Claude 生态的 SKILL.md 会把简介读成 `>` 或 `|`。
fn split_front_matter(text: &str) -> (Vec<(String, String)>, String) {
    let norm = text.replace("\r\n", "\n");
    if let Some(rest) = norm.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let head = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').to_string();
            let lines: Vec<&str> = head.lines().collect();
            let mut meta = Vec::new();
            let mut i = 0;
            while i < lines.len() {
                if let Some((k, v)) = lines[i].split_once(':') {
                    let key = k.trim().to_string();
                    let mut val = v.trim().to_string();
                    if val.starts_with('>') || val.starts_with('|') {
                        let mut buf = String::new();
                        i += 1;
                        while i < lines.len() {
                            let l = lines[i];
                            if l.trim().is_empty() {
                                i += 1;
                                continue;
                            }
                            if !(l.starts_with(' ') || l.starts_with('\t')) {
                                break;
                            }
                            if !buf.is_empty() {
                                buf.push(' ');
                            }
                            buf.push_str(l.trim());
                            i += 1;
                        }
                        val = buf;
                        if !key.is_empty() {
                            meta.push((key, val));
                        }
                        continue;
                    }
                    if !key.is_empty() {
                        meta.push((key, val));
                    }
                }
                i += 1;
            }
            return (meta, body);
        }
    }
    (Vec::new(), norm)
}

/// 文件名 → id 片段：非 ASCII 字母数字折成 `-`。
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "imported".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nw-skills-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn sk(id: &str, detail: &str) -> Skill {
        Skill {
            id: id.into(),
            name: "需求分析".into(),
            summary: "拆需求".into(),
            detail: detail.into(),
            category: SkillCategory::Business,
            author: "t".into(),
            installs: 0,
            stars: 0,
            glyph: "需".into(),
            installed: true,
            kind: SkillKind::Text,
            mcp: None,
        }
    }

    #[test]
    fn install_writes_front_matter_and_body() {
        let root = temp_root("install");
        let file = install_text_in(&root, &sk("community.prd", "## 步骤\n1. 拆用户故事")).unwrap();
        assert!(file.is_file());
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("id: community.prd"));
        assert!(text.contains("category: business"));
        let (meta, body) = split_front_matter(&text);
        assert!(meta.iter().any(|(k, v)| k == "id" && v == "community.prd"));
        assert!(body.contains("拆用户故事"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn install_without_detail_falls_back_to_name_and_summary() {
        let root = temp_root("fallback");
        let file = install_text_in(&root, &sk("community.empty", "")).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("# 需求分析"));
        assert!(text.contains("拆需求"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parse_md_reads_block_scalar_description() {
        let md = "---\nname: academy-guide\ndescription: >\n  This skill should be used when\n  the user wants to learn.\n---\n\n# 正文\n干活。\n";
        let s = parse_md(md, "academy-guide");
        assert_eq!(
            s.summary,
            "This skill should be used when the user wants to learn."
        );

        let md = "---\nname: x\ndescription: |\n  line one\n  line two\n---\n\nbody\n";
        let s = parse_md(md, "x");
        assert_eq!(s.summary, "line one line two");
    }

    #[test]
    fn parse_md_reads_front_matter_and_defaults() {
        let text =
            "---\nid: local.foo\nname: 测试技能\nsummary: 一句话\ncategory: ops\n---\n\n# 正文\n干活。\n";
        let s = parse_md(text, "ignored");
        assert_eq!(s.id, "local.foo");
        assert_eq!(s.name, "测试技能");
        assert_eq!(s.summary, "一句话");
        assert_eq!(s.category, SkillCategory::Ops);
        assert!(s.detail.contains("干活"));

        let s2 = parse_md("# 手写技能\n\n做点事。", "my skill");
        assert_eq!(s2.id, "local.my-skill");
        assert_eq!(s2.name, "手写技能");
        assert_eq!(s2.summary, "做点事。");
        assert_eq!(s2.category, SkillCategory::Dev);
    }

    #[test]
    fn dir_name_blocks_path_escape() {
        assert_eq!(dir_name("community.prd-analysis"), "community.prd-analysis");
        assert_eq!(dir_name("../../evil/x"), "evil_x");
        assert_eq!(dir_name(".."), "skill");
        assert_eq!(dir_name(""), "skill");
        // 连续点会折成 `_.`，不会留下单独的 `..` 目录名
        assert_eq!(dir_name("a..b"), "a._b");
        assert!(!dir_name("a..b").contains(".."));
    }
}
