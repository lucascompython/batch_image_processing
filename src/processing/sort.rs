use rapidhash::{HashSetExt, RapidHashSet as HashSet};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Operation to perform when sorting numbered files.
#[derive(Clone)]
pub enum NumberedSortMode {
    /// Move files into their number folders.
    Move,
    /// Copy files into their number folders, leaving originals in place.
    Copy,
}

/// Configuration for numbered-file sorting.
#[derive(Clone)]
pub struct NumberedSortConfig {
    /// Folder containing the numbered files.
    pub source_dir: PathBuf,
    /// Whether files are moved or copied.
    pub mode: NumberedSortMode,
}

/// A discovered file that should be sorted.
pub struct NumberedSortJob {
    /// Original path of the file.
    pub source: PathBuf,
    /// Parsed leading number, used as the destination directory name.
    pub number: String,
    /// Final destination path.
    pub destination: PathBuf,
}

/// Result of sorting one file.
pub struct NumberedSortResult {
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub number: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

impl NumberedSortResult {
    fn success(job: &NumberedSortJob) -> Self {
        Self {
            source: job.source.clone(),
            destination: Some(job.destination.clone()),
            number: Some(job.number.clone()),
            success: true,
            error: None,
        }
    }

    fn failed(
        source: PathBuf,
        destination: Option<PathBuf>,
        number: Option<String>,
        error: String,
    ) -> Self {
        Self {
            source,
            destination,
            number,
            success: false,
            error: Some(error),
        }
    }
}

/// Aggregate counters for a completed sort run.
pub struct NumberedSortSummary {
    pub discovered: usize,
    pub sorted: usize,
    pub failed: usize,
    pub skipped: usize,
    pub folders_created: usize,
}

/// Sort all numbered files in `config.source_dir`.
///
/// The sorter recognizes a leading numeric group in the file stem. For example,
/// all of these resolve to folder `82`:
///
/// - `82.jpg`
/// - `82 (2).jpg`
/// - `82-001.jpg`
/// - `82-002.jpg`
/// - `82-003.jpg`
/// - `82-002 (2).jpg`
///
/// Directories are ignored. Files whose stem does not start with ASCII digits
/// are skipped.
///
/// The callback receives `(completed, total)` after each file operation.
pub fn sort_numbered_files(
    config: &NumberedSortConfig,
    progress_callback: impl Fn(usize, usize) + Send + Sync,
) -> (NumberedSortSummary, Vec<NumberedSortResult>) {
    let discovery = discover_numbered_file_jobs(&config.source_dir);
    let (jobs, skipped, mut discovery_failures) = match discovery {
        Ok(discovery) => discovery,
        Err(error) => {
            let summary = NumberedSortSummary {
                discovered: 0,
                sorted: 0,
                failed: 1,
                skipped: 0,
                folders_created: 0,
            };

            return (
                summary,
                vec![NumberedSortResult::failed(
                    config.source_dir.clone(),
                    None,
                    None,
                    error,
                )],
            );
        }
    };

    let folders_created = create_destination_folders(&jobs, &mut discovery_failures);

    let total = jobs.len();
    let completed = Arc::new(AtomicUsize::new(0));
    let progress_callback = &progress_callback;

    let mut results: Vec<NumberedSortResult> = jobs
        .par_iter()
        .map(|job| {
            let result = sort_one_file(job, config.mode.clone());
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            progress_callback(done, total);
            result
        })
        .collect();

    results.append(&mut discovery_failures);

    let sorted = results.iter().filter(|result| result.success).count();
    let failed = results.len().saturating_sub(sorted);

    let summary = NumberedSortSummary {
        discovered: total,
        sorted,
        failed,
        skipped,
        folders_created,
    };

    (summary, results)
}

/// Discover sortable files without moving/copying anything.
///
/// Returns:
///
/// - jobs to execute,
/// - skipped file count,
/// - per-entry discovery failures.
pub fn discover_numbered_file_jobs(
    source_dir: &Path,
) -> Result<(Vec<NumberedSortJob>, usize, Vec<NumberedSortResult>), String> {
    let entries = fs::read_dir(source_dir)
        .map_err(|error| format!("Failed to read folder {}: {error}", source_dir.display()))?;

    let mut jobs = Vec::new();
    let mut skipped = 0usize;
    let mut failures = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(NumberedSortResult::failed(
                    source_dir.to_path_buf(),
                    None,
                    None,
                    format!("Failed to read folder entry: {error}"),
                ));
                continue;
            }
        };

        let path = entry.path();

        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                failures.push(NumberedSortResult::failed(
                    path,
                    None,
                    None,
                    format!("Failed to inspect file type: {error}"),
                ));
                continue;
            }
        };

        if !file_type.is_file() {
            skipped += 1;
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            skipped += 1;
            continue;
        };

        let Some(number) = parse_number_folder_name(stem) else {
            skipped += 1;
            continue;
        };

        let Some(file_name) = path.file_name() else {
            skipped += 1;
            continue;
        };

        let destination = source_dir.join(&number).join(file_name);

        jobs.push(NumberedSortJob {
            source: path,
            number,
            destination,
        });
    }

    Ok((jobs, skipped, failures))
}

/// Parse the folder name from a numbered file stem.
///
/// The parser accepts any file stem that starts with one or more ASCII digits.
/// It stops at the first non-digit character, so these all become `82`:
///
/// - `82`
/// - `82 (2)`
/// - `82-001`
/// - `82-002 (2)`
///
/// Stems that do not start with a digit return `None`.
pub fn parse_number_folder_name(file_stem: &str) -> Option<String> {
    let trimmed = file_stem.trim_start();
    let number_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();

    if number_len == 0 {
        return None;
    }

    Some(trimmed[..number_len].to_string())
}

fn create_destination_folders(
    jobs: &[NumberedSortJob],
    failures: &mut Vec<NumberedSortResult>,
) -> usize {
    let mut folders = HashSet::with_capacity(jobs.len().min(4096));

    for job in jobs {
        let Some(parent) = job.destination.parent() else {
            failures.push(NumberedSortResult::failed(
                job.source.clone(),
                Some(job.destination.clone()),
                Some(job.number.clone()),
                "Destination has no parent folder".to_string(),
            ));
            continue;
        };

        folders.insert(parent.to_path_buf());
    }

    let folders_created = Arc::new(AtomicUsize::new(0));

    let folder_results: Vec<_> = folders
        .par_iter()
        .filter_map(|folder| match fs::create_dir_all(folder) {
            Ok(()) => {
                folders_created.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(error) => Some(NumberedSortResult::failed(
                folder.clone(),
                None,
                folder
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
                format!(
                    "Failed to create destination folder {}: {error}",
                    folder.display()
                ),
            )),
        })
        .collect();

    failures.extend(folder_results);

    folders_created.load(Ordering::Relaxed)
}

fn sort_one_file(job: &NumberedSortJob, mode: NumberedSortMode) -> NumberedSortResult {
    if job.destination.exists() {
        return NumberedSortResult::failed(
            job.source.clone(),
            Some(job.destination.clone()),
            Some(job.number.clone()),
            format!("Destination already exists: {}", job.destination.display()),
        );
    }

    let result = match mode {
        NumberedSortMode::Move => fs::rename(&job.source, &job.destination).map(|_| ()),
        NumberedSortMode::Copy => fs::copy(&job.source, &job.destination).map(|_| ()),
    };

    match result {
        Ok(()) => NumberedSortResult::success(job),
        Err(error) => NumberedSortResult::failed(
            job.source.clone(),
            Some(job.destination.clone()),
            Some(job.number.clone()),
            format!(
                "Failed to {} {} to {}: {error}",
                match mode {
                    NumberedSortMode::Move => "move",
                    NumberedSortMode::Copy => "copy",
                },
                job.source.display(),
                job.destination.display()
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_number_folder_name;

    #[test]
    fn parses_numbered_file_stems() {
        assert_eq!(parse_number_folder_name("82").as_deref(), Some("82"));
        assert_eq!(parse_number_folder_name("82 (2)").as_deref(), Some("82"));
        assert_eq!(parse_number_folder_name("82-001").as_deref(), Some("82"));
        assert_eq!(parse_number_folder_name("82-002").as_deref(), Some("82"));
        assert_eq!(parse_number_folder_name("82-003").as_deref(), Some("82"));
        assert_eq!(
            parse_number_folder_name("82-002 (2)").as_deref(),
            Some("82")
        );
    }

    #[test]
    fn rejects_non_numbered_file_stems() {
        assert_eq!(parse_number_folder_name("bike-82"), None);
        assert_eq!(parse_number_folder_name("IMG_0082"), None);
        assert_eq!(parse_number_folder_name(""), None);
        assert_eq!(parse_number_folder_name("  "), None);
    }

    #[test]
    fn trims_leading_whitespace_before_number() {
        assert_eq!(parse_number_folder_name("  82-001").as_deref(), Some("82"));
    }
}
