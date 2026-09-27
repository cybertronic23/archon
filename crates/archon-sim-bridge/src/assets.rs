//! Resolve MuJoCo assets: builtin / local path / remote URL (xml|zip).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AssetCatalog {
    pub version: u32,
    pub builtins: std::collections::BTreeMap<String, BuiltinAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinAsset {
    pub description: String,
    pub mjcf: String,
    #[serde(default)]
    pub camera: Option<String>,
    #[serde(default)]
    pub default_instruction: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedAsset {
    pub model_path: PathBuf,
    pub camera: Option<String>,
    pub source: String,
}

pub fn default_catalog_path() -> PathBuf {
    let candidates = [
        PathBuf::from("python/assets/catalog.json"),
        PathBuf::from("../python/assets/catalog.json"),
        PathBuf::from("../../python/assets/catalog.json"),
    ];
    for c in candidates {
        if c.exists() {
            return fs::canonicalize(&c).unwrap_or(c);
        }
    }
    PathBuf::from("python/assets/catalog.json")
}

pub fn load_catalog(path: &Path) -> Result<AssetCatalog> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read catalog {}", path.display()))?;
    Ok(serde_json::from_str(&text).context("parse catalog.json")?)
}

pub fn list_builtins(catalog: &AssetCatalog) -> Vec<(String, String)> {
    catalog
        .builtins
        .iter()
        .map(|(k, v)| (k.clone(), v.description.clone()))
        .collect()
}

/// Spec: `builtin:name` | local path | `https://...xml|.zip`
pub fn resolve_model_spec(spec: &str, catalog: &AssetCatalog, catalog_path: &Path) -> Result<ResolvedAsset> {
    let python_root = catalog_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .context("catalog should live under python/assets/")?;

    if let Some(name) = spec.strip_prefix("builtin:") {
        let entry = catalog
            .builtins
            .get(name)
            .with_context(|| format!("unknown builtin '{name}'. Try --list-models"))?;
        let path = python_root.join(&entry.mjcf);
        if !path.exists() {
            bail!("builtin mjcf missing: {}", path.display());
        }
        return Ok(ResolvedAsset {
            model_path: fs::canonicalize(&path).unwrap_or(path),
            camera: entry.camera.clone(),
            source: format!("builtin:{name}"),
        });
    }

    if spec.starts_with("http://") || spec.starts_with("https://") {
        let cached = fetch_remote_model(spec)?;
        return Ok(ResolvedAsset {
            model_path: cached,
            camera: None,
            source: spec.to_string(),
        });
    }

    let path = PathBuf::from(spec);
    if !path.exists() {
        bail!("model path not found: {spec}");
    }
    let path = fs::canonicalize(&path).unwrap_or(path);
    if path.is_dir() {
        let xml = find_mjcf_in_dir(&path)?;
        return Ok(ResolvedAsset {
            model_path: xml,
            camera: None,
            source: spec.to_string(),
        });
    }
    Ok(ResolvedAsset {
        model_path: path,
        camera: None,
        source: spec.to_string(),
    })
}

fn cache_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".archon").join("assets").join("cache")
}

fn fetch_remote_model(url: &str) -> Result<PathBuf> {
    let root = cache_root();
    fs::create_dir_all(&root)?;
    let hash = simple_hash(url);
    let dest_dir = root.join(format!("{hash:016x}"));
    fs::create_dir_all(&dest_dir)?;

    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".xml") || lower.contains(".xml?") {
        let out = dest_dir.join("model.xml");
        if !out.exists() {
            download_file(url, &out)?;
        }
        return Ok(out);
    }

    // zip or unknown: download archive then find mjcf
    let archive = dest_dir.join("download.bin");
    if !archive.exists() {
        download_file(url, &archive)?;
    }
    let extract_dir = dest_dir.join("extracted");
    if !extract_dir.exists() {
        fs::create_dir_all(&extract_dir)?;
        extract_archive(&archive, &extract_dir)?;
    }
    find_mjcf_in_dir(&extract_dir)
}

fn download_file(url: &str, out: &Path) -> Result<()> {
    // Prefer curl for large assets / redirects without pulling reqwest into offline workspace.
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(out)
        .arg(url)
        .status()
        .with_context(|| {
            format!(
                "failed to spawn curl to download {url}. Install curl or place the file locally."
            )
        })?;
    if !status.success() {
        bail!("curl failed downloading {url} (status {status})");
    }
    Ok(())
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    // Try unzip then tar.
    let unzip = Command::new("unzip")
        .args(["-q", "-o"])
        .arg(archive)
        .arg("-d")
        .arg(dest)
        .status();
    if let Ok(st) = unzip {
        if st.success() {
            return Ok(());
        }
    }
    let tar = Command::new("tar")
        .args(["-xf"])
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("tar extract")?;
    if !tar.success() {
        bail!(
            "could not extract {} as zip/tar. Provide a .xml MJCF or a zip of MJCF+meshes.",
            archive.display()
        );
    }
    Ok(())
}

fn find_mjcf_in_dir(dir: &Path) -> Result<PathBuf> {
    let mut xmls = Vec::new();
    visit_xml(dir, &mut xmls)?;
    if xmls.is_empty() {
        bail!("no .xml MJCF found under {}", dir.display());
    }
    // Prefer scene.xml / *scene*.xml / shortest path
    xmls.sort_by_key(|p| {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let prefer = if name == "scene.xml" {
            0
        } else if name.contains("scene") {
            1
        } else {
            2
        };
        (prefer, p.components().count(), name)
    });
    Ok(xmls.remove(0))
}

fn visit_xml(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_xml(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("xml") {
            out.push(path);
        }
    }
    Ok(())
}

fn simple_hash(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_builtin() {
        let catalog_path = default_catalog_path();
        if !catalog_path.exists() {
            return;
        }
        let catalog = load_catalog(&catalog_path).unwrap();
        let r = resolve_model_spec("builtin:desktop_arm", &catalog, &catalog_path).unwrap();
        assert!(r.model_path.exists());
        assert!(r.source.starts_with("builtin:"));
    }
}
