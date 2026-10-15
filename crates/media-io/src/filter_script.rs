//! Filtergraph script file for long ffmpeg commands.
//!
//! Windows caps a child-process command line at 32767 characters.
//! Exports (and big previews) with dozens of long-path inputs plus a
//! multi-kilobyte `-filter_complex` graph blow past it, and spawn
//! fails with OS error 206 ("filename or extension is too long").
//! Passing the graph via `-filter_complex_script <file>` moves the
//! biggest chunk out of argv. The file is deleted on drop so a failed
//! spawn never litters the temp dir.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct FilterScript {
    path: PathBuf,
}

impl FilterScript {
    /// Write `filtergraph` to a temp file. Unique per call (pid +
    /// counter + nanos) so concurrent exports/previews never share one.
    pub fn write(filtergraph: &str) -> Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("caprust-fg-{}-{n}-{nanos}.txt", std::process::id()));
        std::fs::write(&path, filtergraph)
            .with_context(|| format!("write filtergraph script {}", path.display()))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Replace the `-filter_complex <graph>` pair in `args` with
/// `-filter_complex_script <tempfile>`, keeping everything else
/// byte-identical. Returns the guard owning the script file, or None
/// when there is no `-filter_complex` pair. The caller must keep the
/// guard alive until the ffmpeg child exits (it deletes the file on
/// drop).
/// Windows caps a child-process command line at 32767 characters.
/// Stay comfortably under it so NULL separators and quoting never push
/// this over the edge.
const MAX_INLINE_CMDLINE: usize = 30_000;

pub fn swap_filter_complex(args: &mut [String]) -> Result<Option<FilterScript>> {
    let Some(pos) = args.iter().position(|a| a == "-filter_complex") else {
        return Ok(None);
    };
    if pos + 1 >= args.len() {
        anyhow::bail!("-filter_complex without a filtergraph argument");
    }

    // Do not force a script file for short commands. Some ffmpeg
    // wrappers/sandboxes reject `-filter_complex_script`, and inline
    // is both simpler and avoids the temp file when argv fits. The
    // script path remains for the long Windows commands it was added
    // for.
    let estimated = args.iter().map(|a| a.len() + 1).sum::<usize>();
    if estimated <= MAX_INLINE_CMDLINE {
        return Ok(None);
    }

    let script = FilterScript::write(&args[pos + 1])?;
    args[pos] = "-/filter_complex".to_string();
    args[pos + 1] = script.path().to_string_lossy().to_string();
    Ok(Some(script))
}

impl Drop for FilterScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_roundtrips_and_cleans_up() {
        let fg = "[0:v]null[v]";
        let path = {
            let s = FilterScript::write(fg).expect("write script");
            let p = s.path().to_path_buf();
            assert_eq!(std::fs::read_to_string(&p).unwrap(), fg);
            p
        };
        assert!(!path.exists(), "script file must be deleted on drop");
    }

    #[test]
    fn concurrent_scripts_do_not_collide() {
        let a = FilterScript::write("a").unwrap();
        let b = FilterScript::write("b").unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn short_command_keeps_filter_complex_inline() {
        let mut args = vec![
            "-y".to_string(),
            "-filter_complex".to_string(),
            "[0:v]null[v]".to_string(),
            "-map".to_string(),
            "[v]".to_string(),
        ];
        let swapped = swap_filter_complex(&mut args).expect("swap");
        assert!(swapped.is_none(), "short commands must stay inline");
        assert_eq!(args[1], "-filter_complex");
        assert_eq!(args[2], "[0:v]null[v]");
    }

    #[test]
    fn long_command_swaps_to_script_and_keeps_rest() {
        let graph = "[0:v]null[v]".repeat(MAX_INLINE_CMDLINE / 12 + 2);
        let mut args = vec![
            "-y".to_string(),
            "-filter_complex".to_string(),
            graph.clone(),
            "-map".to_string(),
            "[v]".to_string(),
        ];
        let guard = swap_filter_complex(&mut args)
            .expect("swap")
            .expect("pair found");
        assert_eq!(args[1], "-/filter_complex");
        assert_eq!(
            std::fs::read_to_string(&args[2]).unwrap().len(),
            graph.len()
        );
        assert_eq!(args[3], "-map");
        assert!(guard.path().exists());
    }

    #[test]
    fn swap_without_pair_is_noop() {
        let mut args = vec!["-y".to_string()];
        assert!(swap_filter_complex(&mut args).unwrap().is_none());
        assert_eq!(args, vec!["-y".to_string()]);
    }
}
