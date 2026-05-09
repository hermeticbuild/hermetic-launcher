use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <tag> <output-file>", args[0]);
        eprintln!("Example: {} v0.2.1 hermetic_launcher-v0.2.1.tar.gz", args[0]);
        std::process::exit(1);
    }

    let _tag = &args[1]; // Tag is used in CLI for documentation but not needed by function
    let output_path = &args[2];

    create_release_archive(output_path)?;

    eprintln!("Created release archive: {}", output_path);
    Ok(())
}

fn create_release_archive(output_path: &str) -> Result<()> {
    let repo_root = find_repo_root()?;
    let output_file = File::create(output_path)
        .with_context(|| format!("Failed to create output file: {}", output_path))?;

    let encoder = GzEncoder::new(output_file, Compression::default());
    let mut archive = tar::Builder::new(encoder);

    // Files to include at the root
    let root_files = ["MODULE.bazel", "LICENSE", "BUILD.bazel"];

    for file in &root_files {
        let file_path = repo_root.join(file);
        let mut header = tar::Header::new_gnu();

        let metadata = std::fs::metadata(&file_path)
            .with_context(|| format!("Failed to read metadata for {}", file))?;
        header.set_size(metadata.len());
        header.set_mode(0o644);
        header.set_cksum();

        let mut file_handle = File::open(&file_path)
            .with_context(|| format!("Failed to open {}", file))?;

        archive.append_data(&mut header, file, &mut file_handle)
            .with_context(|| format!("Failed to add {} to archive", file))?;

        eprintln!("Added: {}", file);
    }

    // Add launcher directory recursively
    let launcher_dir = repo_root.join("launcher");
    add_directory_to_archive(&mut archive, &launcher_dir, "launcher")?;

    // Add e2e directory recursively
    let e2e_dir = repo_root.join("e2e");
    add_directory_to_archive(&mut archive, &e2e_dir, "e2e")?;

    // Finish writing the archive
    archive.finish()
        .context("Failed to finalize archive")?;

    Ok(())
}

fn add_directory_to_archive<W: Write>(
    archive: &mut tar::Builder<W>,
    source_dir: &Path,
    archive_prefix: &str,
) -> Result<()> {
    for entry in WalkDir::new(source_dir)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        // Skip the root directory itself
        if path == source_dir {
            continue;
        }

        let relative_path = path.strip_prefix(source_dir)
            .context("Failed to compute relative path")?;

        let archive_path = PathBuf::from(archive_prefix).join(relative_path);
        let archive_path_str = archive_path.to_str()
            .context("Invalid UTF-8 in path")?
            .replace('\\', "/"); // Normalize path separators for tar

        if entry.file_type().is_dir() {
            // Add directory entry
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();

            archive.append_data(&mut header, &archive_path_str, io::empty())
                .with_context(|| format!("Failed to add directory {}", archive_path_str))?;

            eprintln!("Added: {}/", archive_path_str);
        } else if entry.file_type().is_file() {
            // Add file
            let metadata = entry.metadata()
                .context("Failed to read file metadata")?;
            let mut header = tar::Header::new_gnu();
            header.set_size(metadata.len());
            header.set_mode(if is_executable(&metadata) { 0o755 } else { 0o644 });
            header.set_cksum();

            let mut file_handle = File::open(path)
                .with_context(|| format!("Failed to open {}", path.display()))?;

            archive.append_data(&mut header, &archive_path_str, &mut file_handle)
                .with_context(|| format!("Failed to add file {}", archive_path_str))?;

            eprintln!("Added: {}", archive_path_str);
        }
        // Skip symlinks and other special files
    }

    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn find_repo_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;

    // Walk up from current directory to find the repo root
    let mut dir = current_dir.as_path();
    loop {
        if dir.join("MODULE.bazel").exists() {
            return Ok(dir.to_path_buf());
        }

        dir = dir.parent()
            .context("Could not find repository root (no MODULE.bazel found)")?;
    }
}
