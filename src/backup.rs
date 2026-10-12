use std::path::{Path, PathBuf};

pub fn stamp(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let rem = epoch_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn backup_file(path: &Path, keep: usize) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let dir = crate::paths::backup_dir();
    std::fs::create_dir_all(&dir)?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("backup");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("dat");
    let target = dir.join(format!("{stem}-{}.{ext}.bak", stamp(now_secs())));
    std::fs::copy(path, &target)?;
    prune(&dir, stem, ext, keep)?;
    Ok(Some(target))
}

fn prune(dir: &Path, stem: &str, ext: &str, keep: usize) -> std::io::Result<()> {
    if keep == 0 {
        return Ok(());
    }
    let prefix = format!("{stem}-");
    let suffix = format!(".{ext}.bak");
    let mut matches: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(&suffix))
        })
        .collect();
    if matches.len() <= keep {
        return Ok(());
    }
    matches.sort();
    let excess = matches.len() - keep;
    for old in matches.into_iter().take(excess) {
        std::fs::remove_file(old)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crucial_stamp_formats_known_epoch() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_700_000_000), "20231114-221320");
    }

    #[test]
    fn crucial_stamp_handles_leap_day() {
        assert_eq!(stamp(1_709_164_800), "20240229-000000");
    }

    #[test]
    fn crucial_backup_creates_copy_and_prunes() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };
        let source = dir.join("notes.json");
        std::fs::write(&source, "{}").unwrap();
        let first = backup_file(&source, 2).unwrap().unwrap();
        assert!(first.exists());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&source, "{\"a\":1}").unwrap();
        backup_file(&source, 2).unwrap().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&source, "{\"a\":2}").unwrap();
        backup_file(&source, 2).unwrap().unwrap();
        let count = std::fs::read_dir(dir.join("backups")).unwrap().count();
        assert_eq!(count, 2);
        assert_eq!(backup_file(&dir.join("missing.json"), 2).unwrap(), None);
        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
