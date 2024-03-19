use mdbook::book::{Book, BookItem, Chapter};
use mdbook::errors::Error;
use mdbook::preprocess::PreprocessorContext;
use semver::{Version, VersionReq};

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn is_supported(renderer: &str) -> bool {
    renderer == "html"
}

pub fn run(mut book: Book, context: &PreprocessorContext) -> Result<Book, Error> {
    let book_version = Version::parse(&context.mdbook_version)?;
    let version_req = VersionReq::parse(mdbook::MDBOOK_VERSION)?;

    if !version_req.matches(&book_version) {
        return Err(Error::msg(format!(
            "mdbook-iced plugin version ({}) is not compatible \
            with the book version ({})",
            mdbook::MDBOOK_VERSION,
            context.mdbook_version
        )));
    }

    let revision = context
        .config
        .get_preprocessor("iced")
        .and_then(|table| table.get("rev"))
        .ok_or(Error::msg("mdbook-iced configuration not found"))?;

    let revision = revision
        .as_str()
        .ok_or(Error::msg("`rev` field should be a string"))?;

    let crate_ = set_up_build_crate(&context.root, revision)?;
    let mut hashes = BTreeSet::new();

    for section in &mut book.sections {
        if let BookItem::Chapter(chapter) = section {
            let (content, new_hashes) = process_chapter(&crate_, chapter)?;

            chapter.content = content;
            hashes.extend(new_hashes);
        }
    }

    let output_dir = context.root.join("src").join("icebergs");

    fs::create_dir_all(&output_dir)?;

    clean_output_dir(&crate_.output, &hashes)?;
    clean_output_dir(&output_dir, &hashes)?;

    copy_dir_all(&crate_.output, output_dir)?;

    Ok(book)
}

struct Crate {
    root: PathBuf,
    src: PathBuf,
    output: PathBuf,
    hash: Hash,
}

fn set_up_build_crate(root: &Path, revision: &str) -> Result<Crate, Error> {
    let build_dir = root.to_path_buf().join("icebergs");

    if !build_dir.exists() {
        fs::create_dir(&build_dir)?;
    }

    let cargo_toml = build_dir.join("Cargo.toml");
    let contents = CARGO_TOML.replace("{{ REV }}", revision);

    fs::write(cargo_toml, contents.trim_start())?;

    let src_dir = build_dir.join("src");
    if !src_dir.exists() {
        fs::create_dir(&src_dir)?;
    }

    let output = build_dir.join("output");

    Ok(Crate {
        root: build_dir,
        src: src_dir,
        output,
        hash: Hash::compute(contents.trim_start()),
    })
}

fn process_chapter(crate_: &Crate, chapter: &Chapter) -> Result<(String, BTreeSet<Hash>), Error> {
    use itertools::Itertools;
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    use pulldown_cmark_to_cmark::cmark;

    let events = Parser::new(&chapter.content);

    let mut in_iced_code = false;

    let groups = events.group_by(|event| match event {
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(label)))
            if label.starts_with("rust") && label.split(",").any(|modifier| modifier == "iced") =>
        {
            in_iced_code = true;
            true
        }
        Event::End(TagEnd::CodeBlock) => {
            let is_iced_code = in_iced_code;

            in_iced_code = false;

            is_iced_code
        }
        _ => in_iced_code,
    });

    let mut hashes = BTreeSet::new();

    let output = groups.into_iter().flat_map(|(is_iced_code, group)| {
        if is_iced_code {
            let parts: Vec<_> = group.collect();

            if let Some(Event::Text(code)) = parts.get(1) {
                match compile(crate_, code) {
                    Ok(hash) => {
                        hashes.insert(hash.clone());

                        Box::new(
                            parts
                                .into_iter()
                                .chain(std::iter::once(Event::InlineHtml(embed(hash).into()))),
                        )
                    }
                    Err(_) => Box::new(parts.into_iter()) as Box<dyn Iterator<Item = Event>>,
                }
            } else {
                Box::new(parts.into_iter())
            }
        } else {
            Box::new(group)
        }
    });

    let mut content = String::with_capacity(chapter.content.len());
    let _ = cmark(output, &mut content)?;

    Ok((content, hashes))
}

fn compile(crate_: &Crate, code: &str) -> Result<Hash, Error> {
    use itertools::Itertools;
    use std::process;

    let code = code
        .lines()
        .map(|line| line.strip_prefix("# ").unwrap_or(line))
        .join("\n");

    let hash = Hash::compute(&format!("{}{}", code, crate_.hash.as_str()));
    let output_dir = crate_.output.join(hash.as_str());

    if output_dir.exists() {
        return Ok(hash);
    }

    fs::write(crate_.src.join("main.rs"), code)?;

    let compilation = process::Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .env("RUSTFLAGS", "")
        .current_dir(&crate_.root)
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
            output_dir.to_str().expect("valid path"),
            "target/wasm32-unknown-unknown/release/iceberg.wasm",
        ])
        .current_dir(&crate_.root)
        .spawn()?
        .wait()?;

    Ok(hash)
}

fn clean_output_dir(dir: &Path, hashes: &BTreeSet<Hash>) -> Result<(), Error> {
    let deleted = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| !hashes.iter().any(|hash| hash.as_str() == entry.file_name()));

    for entry in deleted {
        fs::remove_dir_all(entry.path())?;
    }

    Ok(())
}

fn embed(hash: Hash) -> String {
    SCRIPT.replace("{{ HASH }}", hash.as_str()).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Hash(String);

impl Hash {
    fn compute(code: &str) -> Self {
        use itertools::Itertools;
        use sha2::{Digest, Sha256};

        Self(
            Sha256::digest(code)
                .into_iter()
                .map(|byte| format!("{byte:x}"))
                .join(""),
        )
    }

    fn as_str(&self) -> &str {
        &self.0
    }
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

const CARGO_TOML: &'static str = r#"
[package]
name = "iceberg"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies.iced]
git = "https://github.com/iced-rs/iced.git"
rev = "{{ REV }}"
features = ["webgl"]
"#;

const SCRIPT: &'static str = r#"
<script type="module" id="iceberg-script-{{ HASH }}">
  import init from './icebergs/{{ HASH }}/iceberg.js';

  let me = document.getElementById('iceberg-script-{{ HASH }}');

  // TODO: Find a more reliable way to find the code block
  let code = me.previousSibling.previousSibling;
  let buttons = code.querySelector('.buttons');
  let play = document.createElement('button');

  async function run() {
    play.remove();

    let example = document.createElement('div');
    example.style.height = "300px";

    let iced = document.createElement('div');
    iced.id = 'iced';

    example.append(iced);
    code.append(example);

    await init();
  }

  play.title = 'Run example';
  play.onclick = run;

  play.classList.add('fa');
  play.classList.add('fa-play');

  buttons.prepend(play);
</script>
"#;
