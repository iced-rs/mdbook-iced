use anyhow::Error;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{self, AtomicU64};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiler {
    build: PathBuf,
    src: PathBuf,
    artifacts: PathBuf,
    hash: u64,
}

impl Compiler {
    pub fn set_up(root: impl AsRef<Path>, reference: Reference) -> Result<Self, Error> {
        const CARGO_TOML: &'static str = include_str!("compiler/Cargo.toml.template");

        let build = root.as_ref().join("target").join("icebergs");
        fs::create_dir_all(&build)?;

        let src = build.join("src");
        fs::create_dir_all(&src)?;

        let artifacts = build.join("target").join("mdbook");
        fs::create_dir_all(&src)?;

        let cargo_config = CARGO_TOML.replace(
            "{{ GIT_REFERENCE }}",
            &match reference {
                Reference::Revision(revision) => format!("rev = \"{revision}\""),
                Reference::Branch(branch) => format!("branch = \"{branch}\""),
                Reference::Tag(tag) => format!("tag = \"{tag}\""),
            },
        );
        fs::write(build.join("Cargo.toml"), cargo_config.trim_start())?;

        let hash = {
            use std::hash::{DefaultHasher, Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            cargo_config.hash(&mut hasher);
            hasher.finish()
        };

        Ok(Self {
            build,
            src,
            artifacts,
            hash,
        })
    }

    pub fn compile(&self, code: &str) -> Result<Iceberg, Error> {
        use itertools::Itertools;
        use sha2::{Digest, Sha256};

        let code = code
            .lines()
            .map(|line| line.strip_prefix("# ").unwrap_or(line))
            .join("\n");

        let hash = Hash(
            Sha256::digest(&format!("{code}{}", self.hash))
                .into_iter()
                .map(|byte| format!("{byte:x}"))
                .join(""),
        );

        let artifact_dir = self.artifacts.join(hash.as_str());

        if artifact_dir.exists() {
            return Ok(Iceberg { hash });
        }

        fs::write(self.src.join("main.rs"), code)?;

        let compilation = process::Command::new("cargo")
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .env("RUSTFLAGS", "")
            .current_dir(&self.build)
            .stdout(process::Stdio::piped())
            .spawn()?;

        std::io::copy(
            &mut std::io::BufReader::new(compilation.stdout.expect("Open compilation output")),
            &mut std::io::stderr(),
        )?;

        process::Command::new("wasm-bindgen")
            .args([
                "--target",
                "web",
                "--no-typescript",
                "--out-dir",
                artifact_dir.to_str().expect("valid artifact path"),
                "target/wasm32-unknown-unknown/release/iceberg.wasm",
            ])
            .current_dir(&self.build)
            .spawn()?
            .wait()?;

        Ok(Iceberg { hash })
    }

    pub fn retain(&self, icebergs: &BTreeSet<Iceberg>) -> Result<(), Error> {
        clean_dir(&self.artifacts, &icebergs)?;

        Ok(())
    }

    pub fn release<'a>(
        &self,
        icebergs: &BTreeSet<Iceberg>,
        target: impl AsRef<Path>,
    ) -> Result<(), Error> {
        let target = target.as_ref();
        clean_dir(target, &icebergs)?;

        for iceberg in icebergs {
            let output = target.join(iceberg.hash.as_str());

            if !output.exists() {
                let artifact = self.artifacts.join(iceberg.hash.as_str());

                copy_dir_all(artifact, output)?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Iceberg {
    hash: Hash,
}

impl Iceberg {
    pub const LIBRARY: &'static str = include_str!("compiler/library.html");
    pub const EMBED: &'static str = include_str!("compiler/embed.html");

    pub fn embed(&self, height: Option<&str>) -> String {
        static COUNT: AtomicU64 = AtomicU64::new(0);

        Self::EMBED
            .replace("{{ HASH }}", self.hash.as_str())
            .replace(
                "{{ ID }}",
                &COUNT.fetch_add(1, atomic::Ordering::Relaxed).to_string(),
            )
            .replace("{{ HEIGHT }}", height.unwrap_or("200px"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reference {
    Revision(String),
    Branch(String),
    Tag(String),
}

impl Reference {
    pub fn parse(table: &toml::value::Table) -> Result<Self, Error> {
        if let Some(toml::Value::String(revision)) = table.get("rev") {
            return Ok(Self::Revision(revision.clone()));
        }

        if let Some(toml::Value::String(branch)) = table.get("branch") {
            return Ok(Self::Branch(branch.clone()));
        }

        if let Some(toml::Value::String(tag)) = table.get("tag") {
            return Ok(Self::Tag(tag.clone()));
        }

        Err(Error::msg(
            "No Git reference found for `iced` in the preprocessor configuration. \
            Please, specify a `rev`, `branch` or `tag`.",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(String);

impl Hash {
    fn as_str(&self) -> &str {
        &self.0
    }
}

fn clean_dir(dir: impl AsRef<Path>, icebergs: &BTreeSet<Iceberg>) -> Result<(), Error> {
    let deleted = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            !icebergs
                .iter()
                .any(|iceberg| iceberg.hash.as_str() == entry.file_name())
        });

    for entry in deleted {
        fs::remove_dir_all(entry.path())?;
    }

    Ok(())
}

fn copy_dir_all(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), Error> {
    fs::create_dir_all(&to)?;

    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.as_ref().join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir_all(entry.path(), target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}
