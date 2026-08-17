#![allow(missing_docs)]
//! Skill manager — loads, stores, and manages skills.

use super::frontmatter::parse_skill;
use super::prompt::{compact_path, format_skills_for_prompt};
use super::requirements::check_requirements;
use super::types::*;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

pub struct SkillManager {
    skills_dir: PathBuf,
    bundled_dir: PathBuf,
    /// Foundation lockfile + archive dir (RFC-048 §4). `None` until
    /// [`SkillManager::with_foundation`] attaches a home.
    foundation_lock: Option<PathBuf>,
    foundation_packages_dir: Option<PathBuf>,
    installed: RwLock<HashMap<String, SkillEntry>>,
}
impl std::fmt::Debug for SkillManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillManager")
            .field("skills_dir", &self.skills_dir)
            .field("bundled_dir", &self.bundled_dir)
            .finish()
    }
}
impl SkillManager {
    pub fn new(skills_dir: PathBuf, bundled_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            bundled_dir,
            foundation_lock: None,
            foundation_packages_dir: None,
            installed: RwLock::new(HashMap::new()),
        }
    }

    /// Attach the Foundation shared-package registry (RFC-048 §4).
    ///
    /// `home` is the user home holding `~/.oxi/foundation/v1`. After this,
    /// [`Self::init`] loads verified packages between the bundled defaults
    /// and the user skills dir — bundled < Foundation < user/workspace.
    pub fn with_foundation(mut self, home: &Path) -> Self {
        self.foundation_lock = Some(crate::foundation::packages::default_lock_path(home));
        self.foundation_packages_dir = Some(crate::foundation::packages_dir(home));
        self
    }
    pub async fn init(&self) -> Result<()> {
        if !self.skills_dir.exists() {
            tokio::fs::create_dir_all(&self.skills_dir).await?;
        }
        if self.is_dir_empty(&self.skills_dir).await? && self.bundled_dir.exists() {
            self.bootstrap_from_bundled().await?;
        }
        let mut map: HashMap<String, SkillEntry> = HashMap::new();
        if self.bundled_dir.exists() {
            self.load_skills_from_dir(&self.bundled_dir, true, &mut map)
                .await?;
        }
        // RFC-048 §4 precedence: bundled defaults first, then verified
        // Foundation packages, then user/workspace skills (later wins).
        self.load_foundation_packages(&mut map).await?;
        self.load_skills_from_dir(&self.skills_dir, false, &mut map)
            .await?;
        *self.installed.write().await = map;
        Ok(())
    }
    pub async fn list_skills(&self) -> Vec<SkillEntry> {
        let mut s: Vec<SkillEntry> = self.installed.read().await.values().cloned().collect();
        s.sort_by(|a, b| a.skill.name.cmp(&b.skill.name));
        s
    }
    pub async fn get_skill(&self, name: &str) -> Option<SkillEntry> {
        self.installed.read().await.get(name).cloned()
    }
    pub async fn get_skill_content(&self, name: &str) -> Option<String> {
        self.installed
            .read()
            .await
            .get(name)
            .map(|e| e.skill.content.clone())
    }
    pub async fn build_snapshot(
        &self,
        _agent_id: Option<&str>,
        skill_filter: Option<&[String]>,
    ) -> SkillSnapshot {
        self.build_snapshot_for(_agent_id, skill_filter, None).await
    }

    /// Persona-aware snapshot (RFC-048 §4). Foundation packages that
    /// declare a persona hint contribute content only to agents running
    /// that persona — a request never receives every shared `SKILL.md`.
    /// Every contributing package's verified digest is recorded for audit.
    pub async fn build_snapshot_for(
        &self,
        _agent_id: Option<&str>,
        skill_filter: Option<&[String]>,
        persona: Option<&str>,
    ) -> SkillSnapshot {
        let entries = self.list_skills().await;
        let visible: Vec<&SkillEntry> = entries
            .iter()
            .filter(|e| {
                e.status != SkillStatus::Disabled
                    && e.eligibility.eligible
                    && !e.invocation.disable_model_invocation
                    && persona_hint_matches(persona, e.foundation.as_ref())
            })
            .collect();
        let filtered: Vec<&SkillEntry> = if let Some(f) = skill_filter {
            visible
                .into_iter()
                .filter(|e| f.contains(&e.skill.name))
                .collect()
        } else {
            visible
        };
        let foundation_packages = {
            let mut seen = std::collections::BTreeSet::new();
            filtered
                .iter()
                .filter_map(|e| e.foundation.clone())
                .filter(|f| seen.insert(f.id.clone()))
                .collect()
        };
        SkillSnapshot {
            prompt: format_skills_for_prompt(&filtered),
            skills: filtered
                .iter()
                .map(|e| SkillRef {
                    name: e.skill.name.clone(),
                    description: e.skill.description.clone(),
                    file_path: compact_path(&e.skill.file_path),
                    primary_env: e.metadata.as_ref().and_then(|m| m.primary_env.clone()),
                    required_env: e
                        .metadata
                        .as_ref()
                        .map(|m| m.requires.env.clone())
                        .unwrap_or_default(),
                    required_integrations: e
                        .metadata
                        .as_ref()
                        .map(|m| m.requires.integrations.clone())
                        .unwrap_or_default(),
                })
                .collect(),
            skill_filter: skill_filter.map(|f| f.to_vec()),
            foundation_packages,
        }
    }
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        let mut installed = self.installed.write().await;
        if let Some(entry) = installed.get_mut(name) {
            // Foundation packages are immutable (RFC-048 §4) — no in-place
            // state writes into the shared registry tree.
            if entry.foundation.is_some() {
                anyhow::bail!(
                    "skill '{name}' comes from a read-only Foundation package; \
                     import a local copy to customize it"
                );
            }
            let state = SkillState {
                enabled,
                installed_at: chrono::Utc::now().to_rfc3339(),
                last_modified: chrono::Utc::now().to_rfc3339(),
            };
            tokio::fs::write(
                entry.skill.base_dir.join("state.json"),
                serde_json::to_string_pretty(&state)?,
            )
            .await?;
            entry.status = if enabled {
                if entry.eligibility.eligible {
                    SkillStatus::Ready
                } else {
                    SkillStatus::NeedsSetup
                }
            } else {
                SkillStatus::Disabled
            };
        } else {
            anyhow::bail!("skill not found: {name}");
        }
        Ok(())
    }
    pub async fn create_skill(&self, name: &str, description: &str, content: &str) -> Result<()> {
        let dir = self.skills_dir.join(name);
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n{content}"),
        )
        .await?;
        let entry = Self::load_skill_entry(&dir.join("SKILL.md"), false)?;
        self.installed.write().await.insert(name.to_string(), entry);
        Ok(())
    }
    pub async fn delete_skill(&self, name: &str) -> Result<()> {
        let dir = self.skills_dir.join(name);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        self.installed.write().await.remove(name);
        Ok(())
    }
    /// Write a skill's raw `SKILL.md` content verbatim (frontmatter preserved)
    /// and reindex it.
    ///
    /// Unlike [`create_skill`], this does **not** re-synthesize the frontmatter
    /// from `name`+`description` — the raw bytes are stored untouched so rich
    /// metadata (`requires`, `install`, `allowed-tools`, …) survives. Use this
    /// for inline edits and `.md` text imports.
    pub async fn write_skill_raw(&self, name: &str, raw: &str) -> Result<SkillEntry> {
        let dir = self.skills_dir.join(name);
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(dir.join("SKILL.md"), raw).await?;
        let entry = Self::load_skill_entry(&dir.join("SKILL.md"), false)?;
        self.installed
            .write()
            .await
            .insert(name.to_string(), entry.clone());
        Ok(entry)
    }

    /// Import a skill from raw `SKILL.md` text (frontmatter preserved).
    ///
    /// Validates the content by parsing it, derives the canonical name from
    /// the frontmatter (falling back to `name_hint`, then erroring), and
    /// writes the content verbatim via [`write_skill_raw`]. Used for the
    /// text-paste and URL import modes.
    pub async fn import_skill_text(
        &self,
        content: &str,
        name_hint: Option<&str>,
    ) -> Result<SkillEntry> {
        let (parsed, _) = parse_skill(content, &self.skills_dir)?;
        let name = if !parsed.name.is_empty() {
            parsed.name.clone()
        } else if let Some(hint) = name_hint {
            sanitize_skill_name(hint)
        } else {
            anyhow::bail!("skill has no name in its frontmatter and none was provided");
        };
        self.write_skill_raw(&name, content).await
    }

    /// Import a skill from a `.zip` / `.skill` archive's raw bytes.
    ///
    /// Extracts into a temp dir under `skills_dir` (same filesystem, so the
    /// final rename is atomic), derives the canonical name from the parsed
    /// `SKILL.md` frontmatter (falling back to `name_hint`), moves the
    /// extracted tree to `skills_dir/{name}/`, records provenance under
    /// `.imported/origin.json`, and reindexes.
    pub async fn import_skill_zip(&self, name_hint: &str, bytes: &[u8]) -> Result<SkillEntry> {
        use std::io::Cursor;

        // 1. Extract into a temp sibling of the target (same FS → atomic rename).
        let tmp_root = self
            .skills_dir
            .join(format!(".import-tmp-{}", uuid::Uuid::new_v4()));
        let extract_dir = tmp_root.join("extract");
        tokio::fs::create_dir_all(&extract_dir).await?;
        {
            let cursor = Cursor::new(bytes);
            let mut zip = zip::ZipArchive::new(cursor).context("read zip archive")?;
            crate::skill::archive::extract_skill_zip(&mut zip, &extract_dir)?;
        }

        // 2. Locate SKILL.md, read + parse for the canonical name.
        let result = self
            .finalize_import(&extract_dir, name_hint, &tmp_root)
            .await;

        // Best-effort cleanup of the temp dir on any outcome.
        let _ = tokio::fs::remove_dir_all(&tmp_root).await;

        result
    }

    /// Move an already-extracted skill tree into `skills_dir/{name}/`,
    /// derive the name from frontmatter, write provenance, and reindex.
    async fn finalize_import(
        &self,
        extract_dir: &Path,
        name_hint: &str,
        tmp_root: &Path,
    ) -> Result<SkillEntry> {
        let skill_md = crate::skill::archive::find_skill_md(extract_dir)
            .context("no SKILL.md found in archive")?;
        let raw = std::fs::read_to_string(&skill_md)
            .with_context(|| format!("reading {}", skill_md.display()))?;
        let (parsed, _) = parse_skill(&raw, extract_dir)?;
        let name = if !parsed.name.is_empty() {
            parsed.name.clone()
        } else {
            sanitize_skill_name(name_hint)
        };

        // 3. Move the extracted tree to skills_dir/{name}.
        let target = self.skills_dir.join(&name);
        if target.exists() {
            tokio::fs::remove_dir_all(&target)
                .await
                .context("remove existing skill")?;
        }
        // Rename extract_dir → target (same filesystem as skills_dir).
        std::fs::rename(extract_dir, &target)
            .with_context(|| format!("moving imported skill to {}", target.display()))?;
        // Drop the now-empty temp root.
        let _ = std::fs::remove_dir_all(tmp_root);

        // 4. Provenance.
        let origin_dir = target.join(".imported");
        tokio::fs::create_dir_all(&origin_dir).await?;
        let origin = serde_json::json!({
            "source": "file",
            "format": format!("{:?}", parsed.format).to_lowercase(),
            "imported_at": chrono::Utc::now().to_rfc3339(),
        });
        tokio::fs::write(origin_dir.join("origin.json"), origin.to_string()).await?;

        // 5. Reindex.
        let entry = Self::load_skill_entry(&target.join("SKILL.md"), false)?;
        self.installed
            .write()
            .await
            .insert(name.clone(), entry.clone());
        Ok(entry)
    }
    pub async fn list_skills_meta(&self) -> Vec<SkillMeta> {
        let mut m: Vec<SkillMeta> = self
            .installed
            .read()
            .await
            .values()
            .map(|e| SkillMeta::from(&e.skill))
            .collect();
        m.sort_by(|a, b| a.name.cmp(&b.name));
        m
    }
    pub async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        Ok(self
            .installed
            .read()
            .await
            .get(name)
            .map(|e| e.skill.clone()))
    }
    pub fn path(&self) -> &PathBuf {
        &self.skills_dir
    }

    /// Load additional skills from an external directory (e.g. bundled defaults).
    /// Each subdirectory containing a `SKILL.md` is loaded as a bundled skill.
    pub async fn load_from_dir(&self, dir: &Path) -> Result<()> {
        let mut map = self.installed.write().await;
        self.load_skills_from_dir(dir, true, &mut map).await?;
        Ok(())
    }

    async fn load_skills_from_dir(
        &self,
        dir: &Path,
        bundled: bool,
        map: &mut HashMap<String, SkillEntry>,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        let mut entries = tokio::fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                let sf = path.join("SKILL.md");
                if sf.exists() {
                    match Self::load_skill_entry(&sf, bundled) {
                        Ok(se) => {
                            let n = se.skill.name.clone();
                            if bundled && map.contains_key(&n) {
                                continue;
                            }
                            map.insert(n, se);
                        }
                        Err(e) => {
                            tracing::warn!("failed to parse skill {:?}: {}", sf, e);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn load_skill_entry(skill_file: &Path, bundled: bool) -> Result<SkillEntry> {
        let content = std::fs::read_to_string(skill_file)
            .with_context(|| format!("reading {skill_file:?}"))?;
        let skill_dir = skill_file.parent().context("no parent")?;
        let (parsed, body) = parse_skill(&content, skill_dir)?;
        let name = if parsed.name.is_empty() {
            skill_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            parsed.name
        };
        let base_dir = skill_dir.parent().context("no grandparent")?.to_path_buf();
        let skill = Skill {
            name: name.clone(),
            description: parsed.description,
            content: body,
            path: skill_file.to_path_buf(),
            base_dir,
            file_path: skill_file.to_path_buf(),
        };
        let (eligibility, status) = {
            let c = check_requirements(&parsed.metadata);
            let elig = c.eligible;
            (
                c,
                if elig {
                    SkillStatus::Ready
                } else {
                    SkillStatus::NeedsSetup
                },
            )
        };
        let status = {
            let sp = skill
                .path
                .parent()
                .expect("skill path has a parent directory")
                .join("state.json");
            if sp.exists() {
                if let Ok(sc) = std::fs::read_to_string(&sp) {
                    if let Ok(s) = serde_json::from_str::<SkillState>(&sc) {
                        if !s.enabled {
                            SkillStatus::Disabled
                        } else {
                            status
                        }
                    } else {
                        status
                    }
                } else {
                    status
                }
            } else {
                status
            }
        };
        Ok(SkillEntry {
            skill,
            metadata: Some(parsed.metadata),
            eligibility,
            status,
            bundled,
            source: if bundled {
                SkillSource::Bundled
            } else {
                SkillSource::Managed
            },
            foundation: None,
            invocation: parsed.invocation,
            format: parsed.format,
            raw_yaml: parsed.raw_yaml,
        })
    }

    /// Load verified Foundation shared packages into the skill map
    /// (RFC-048 §4). Read-only: never writes the lockfile or the
    /// Foundation tree. A missing lock is not an error — Foundation is
    /// optional. Each package is gated individually through
    /// [`crate::foundation::packages::PackageLock::import`] so one bad
    /// digest never hides the healthy packages behind it.
    async fn load_foundation_packages(&self, map: &mut HashMap<String, SkillEntry>) -> Result<()> {
        let Some(lock_path) = self.foundation_lock.clone() else {
            return Ok(());
        };
        let raw = match tokio::fs::read_to_string(&lock_path).await {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, path = %lock_path.display(), "foundation lock unreadable");
                return Ok(());
            }
        };
        let lock = match crate::foundation::packages::PackageLock::parse(&raw) {
            Ok(lock) => lock,
            Err(e) => {
                tracing::warn!(error = %e, "foundation package lock rejected; skipping");
                return Ok(());
            }
        };
        let dir = self.foundation_packages_dir.clone().unwrap_or_else(|| {
            lock_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        });
        for entry in &lock.entries {
            let archive_path = dir.join(format!("{}.zip", entry.id));
            let bytes = match std::fs::read(&archive_path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(error = %e, path = %archive_path.display(), "foundation package archive missing; skipped");
                    continue;
                }
            };
            // Digest/trust gate through the same importer the CLI uses.
            let single = crate::foundation::packages::PackageLock {
                schema_version: lock.schema_version,
                entries: vec![entry.clone()],
            };
            let mut archives = BTreeMap::new();
            archives.insert(entry.id.clone(), bytes.clone());
            let imported = match single.import(&archives) {
                Ok(mut imported) => imported.remove(0),
                Err(e) => {
                    tracing::warn!(error = %e, id = %entry.id, "foundation package rejected");
                    continue;
                }
            };
            match Self::load_foundation_entry(&imported, &bytes, &dir) {
                Ok(skill_entry) => {
                    tracing::info!(id = %entry.id, digest = %entry.digest, "foundation package loaded");
                    map.insert(skill_entry.skill.name.clone(), skill_entry);
                }
                Err(e) => {
                    tracing::warn!(error = %e, id = %entry.id, "foundation package skill parse failed");
                }
            }
        }
        Ok(())
    }

    /// Parse one verified Foundation package archive into a skill entry.
    /// Entirely in memory — the Foundation tree stays read-only.
    fn load_foundation_entry(
        pkg: &crate::foundation::packages::ImportedPackage,
        bytes: &[u8],
        dir: &Path,
    ) -> Result<SkillEntry> {
        use std::io::Read;

        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).context("read foundation package archive")?;
        // Prefer the shallowest SKILL.md (package root) over nested copies.
        let mut skill_md: Option<String> = None;
        for i in 0..zip.len() {
            let name = zip.by_index(i)?.name().to_string();
            if name.ends_with("SKILL.md")
                && skill_md
                    .as_deref()
                    .is_none_or(|best| name.len() < best.len())
            {
                skill_md = Some(name);
            }
        }
        let entry_name = skill_md.context("no SKILL.md found in foundation package")?;
        let mut raw = String::new();
        zip.by_name(&entry_name)
            .with_context(|| format!("extract {entry_name}"))?
            .read_to_string(&mut raw)?;

        let pkg_dir = dir.join(&pkg.id);
        let (parsed, body) = parse_skill(&raw, &pkg_dir)?;
        let name = if parsed.name.is_empty() {
            sanitize_skill_name(&pkg.id)
        } else {
            parsed.name.clone()
        };
        let (eligibility, status) = {
            let check = check_requirements(&parsed.metadata);
            let status = if check.eligible {
                SkillStatus::Ready
            } else {
                SkillStatus::NeedsSetup
            };
            (check, status)
        };
        let skill_file = pkg_dir.join("SKILL.md");
        Ok(SkillEntry {
            skill: Skill {
                name,
                description: parsed.description,
                content: body,
                path: skill_file.clone(),
                base_dir: pkg_dir,
                file_path: skill_file,
            },
            metadata: Some(parsed.metadata),
            eligibility,
            status,
            bundled: false,
            source: SkillSource::Foundation,
            foundation: Some(FoundationPackageInfo {
                id: pkg.id.clone(),
                version: pkg.version.clone(),
                digest: pkg.digest.clone(),
                persona: pkg.persona.clone(),
            }),
            invocation: parsed.invocation,
            format: parsed.format,
            raw_yaml: parsed.raw_yaml,
        })
    }
    async fn bootstrap_from_bundled(&self) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.bundled_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let src = entry.path();
            if src.is_dir() {
                let name = src
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let dest = self.skills_dir.join(name);
                tokio::fs::create_dir_all(&dest).await?;
                let sf = src.join("SKILL.md");
                if sf.exists() {
                    let df = dest.join("SKILL.md");
                    if !df.exists() {
                        tokio::fs::write(&df, tokio::fs::read_to_string(&sf).await?).await?;
                    }
                }
            }
        }
        Ok(())
    }
    async fn is_dir_empty(&self, dir: &Path) -> Result<bool> {
        if !dir.exists() {
            return Ok(true);
        }
        let mut e = tokio::fs::read_dir(dir).await?;
        Ok(e.next_entry().await?.is_none())
    }
}

/// Normalize an arbitrary string into a valid skill directory name:
/// lowercase ascii, digits, and hyphens only. Used as a fallback when an
/// imported skill has no `name` in its frontmatter.
fn sanitize_skill_name(raw: &str) -> String {
    let stem = raw.rsplit(['/', '.']).next().unwrap_or(raw);
    let s: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() && c.is_ascii_lowercase() {
                c
            } else if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "imported-skill".to_string()
    } else {
        s
    }
}

/// RFC-048 §4 persona gating: a Foundation package that declares a
/// persona hint contributes content only when the requesting persona
/// matches (case-insensitive). Packages without a hint are general.
fn persona_hint_matches(persona: Option<&str>, foundation: Option<&FoundationPackageInfo>) -> bool {
    match foundation.and_then(|f| f.persona.as_deref()) {
        None => true,
        Some(hint) => persona.is_some_and(|p| p.eq_ignore_ascii_case(hint)),
    }
}
