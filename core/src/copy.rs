//! Workspace for folders that are not git repos: agents work on a copy, and changes are copied back
//! only when you approve. A manifest of file hashes taken at the start tells what the agents changed,
//! and whether you changed the same file meanwhile (a conflict).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Build output and caches: not copied, agents can regenerate them.
const SKIP: [&str; 14] =
    [".git", ".duo", ".next", ".nuxt", ".turbo", ".cache", "dist", "build", "out", "coverage", "target", "__pycache__", ".pytest_cache", ".mypy_cache"];
/// Dependencies: linked (a junction on Windows), not copied, so tests and builds still run in the copy.
const LINK: [&str; 3] = ["node_modules", ".venv", "venv"];

/// relative path (forward slashes) → content hash
pub type Manifest = HashMap<String, u64>;

// ponytail: std's 64-bit SipHash, not sha1; the manifest only lives in memory to spot changes.
fn hash(file: &Path) -> Option<u64> {
    let bytes = std::fs::read(file).ok()?;
    let mut h = std::hash::DefaultHasher::new();
    bytes.hash(&mut h);
    Some(h.finish())
}

fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root).unwrap_or(file).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if SKIP.contains(&name.as_ref()) || LINK.contains(&name.as_ref()) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() || is_junction(&e.path()) {
            continue;
        }
        if ft.is_dir() {
            walk(&e.path(), out)
        } else if ft.is_file() {
            out.push(e.path())
        }
    }
}

#[cfg(windows)]
fn is_junction(p: &Path) -> bool {
    junction::exists(p).unwrap_or(false)
}
#[cfg(not(windows))]
fn is_junction(_: &Path) -> bool {
    false
}

pub fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    walk(root, &mut out);
    out
}

/// Copy `src` into `dest` and return the hashes of what was copied.
pub fn snapshot(src: &Path, dest: &Path) -> std::io::Result<Manifest> {
    let mut m = Manifest::new();
    std::fs::create_dir_all(dest)?;
    for file in list_files(src) {
        let r = rel(src, &file);
        let to = dest.join(&r);
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::copy(&file, &to)?;
        if let Some(h) = hash(&file) {
            m.insert(r, h);
        }
    }
    for name in LINK {
        let from = src.join(name);
        if from.is_dir() {
            // agents can still install dependencies themselves if this fails
            #[cfg(windows)]
            let _ = junction::create(&from, dest.join(name));
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(&from, dest.join(name));
        }
    }
    Ok(m)
}

#[derive(Debug, Default, Clone)]
pub struct Changes {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    /// you changed these in the original folder too; they are not applied
    pub conflicts: Vec<String>,
    pub insertions: usize,
    pub deletions: usize,
}

impl Changes {
    pub fn file_count(&self) -> usize {
        self.added.len() + self.modified.len() + self.deleted.len()
    }

    pub fn describe(&self) -> String {
        let a = self.added.iter().map(|f| format!("A  {f}"));
        let m = self.modified.iter().map(|f| format!("M  {f}"));
        let d = self.deleted.iter().map(|f| format!("D  {f}"));
        let c = self.conflicts.iter().map(|f| format!("!  {f}  (you changed it too, not applied)"));
        a.chain(m).chain(d).chain(c).take(30).collect::<Vec<_>>().join("\n")
    }
}

fn read_text(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned()
}

fn count(s: &str) -> HashMap<&str, usize> {
    let mut m = HashMap::new();
    for l in s.split('\n') {
        *m.entry(l).or_default() += 1;
    }
    m
}

fn line_diff(a: &str, b: &str) -> (usize, usize) {
    let (ca, cb) = (count(a), count(b));
    let ins = cb.iter().map(|(l, n)| n.saturating_sub(*ca.get(l).unwrap_or(&0))).sum();
    let del = ca.iter().map(|(l, n)| n.saturating_sub(*cb.get(l).unwrap_or(&0))).sum();
    (ins, del)
}

pub fn changes(src: &Path, work: &Path, m: &Manifest) -> Changes {
    let mut c = Changes::default();
    let mut seen = HashSet::new();
    for file in list_files(work) {
        let r = rel(work, &file);
        seen.insert(r.clone());
        let h = hash(&file);
        let before = m.get(&r).copied();
        if before.is_some() && before == h {
            continue;
        }
        let orig = src.join(&r);
        let orig_now = if orig.exists() { hash(&orig) } else { None };
        if orig_now != before {
            c.conflicts.push(r);
            continue;
        }
        let old = if before.is_none() { String::new() } else { read_text(&orig) };
        let (ins, del) = line_diff(&old, &read_text(&file));
        if before.is_none() { c.added.push(r) } else { c.modified.push(r) }
        c.insertions += ins;
        c.deletions += del;
    }
    let mut gone: Vec<_> = m.iter().filter(|(r, _)| !seen.contains(*r)).collect();
    gone.sort();
    for (r, h) in gone {
        let orig = src.join(r);
        if !orig.exists() {
            continue;
        }
        if hash(&orig) != Some(*h) {
            c.conflicts.push(r.clone());
            continue;
        }
        c.deleted.push(r.clone());
        c.deletions += read_text(&orig).split('\n').count();
    }
    c
}

/// Copy the agents' changes into the original folder, and record them as the new baseline.
pub fn apply(src: &Path, work: &Path, c: &Changes, m: &mut Manifest) -> std::io::Result<()> {
    for r in c.added.iter().chain(&c.modified) {
        let to = src.join(r);
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::copy(work.join(r), &to)?;
        if let Some(h) = hash(&to) {
            m.insert(r.clone(), h);
        }
    }
    for r in &c.deleted {
        let _ = std::fs::remove_file(src.join(r));
        m.remove(r);
    }
    Ok(())
}

/// Put one file in the copy back as it was at the start (a new file is removed).
pub fn revert(src: &Path, work: &Path, r: &str, m: &Manifest) {
    let to = work.join(r);
    let Some(h) = m.get(r) else {
        let _ = std::fs::remove_file(&to);
        return;
    };
    let orig = src.join(r);
    if hash(&orig) != Some(*h) {
        return; // you changed it meanwhile: a conflict, never applied anyway
    }
    let _ = std::fs::create_dir_all(to.parent().unwrap());
    let _ = std::fs::copy(orig, to);
}
