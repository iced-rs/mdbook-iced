mod compiler;

use compiler::Compiler;

use mdbook::book::{Book, BookItem, Chapter};
use mdbook::errors::Error;
use mdbook::preprocess::PreprocessorContext;
use semver::{Version, VersionReq};

use std::collections::BTreeSet;
use std::fs;

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

    let config = context
        .config
        .get_preprocessor("iced")
        .ok_or(Error::msg("mdbook-iced configuration not found"))?;

    let reference = compiler::Reference::parse(config)?;
    let compiler = Compiler::set_up(&context.root, reference)?;

    let mut icebergs = BTreeSet::new();

    for section in &mut book.sections {
        if let BookItem::Chapter(chapter) = section {
            let (content, new_icebergs) = process_chapter(&compiler, chapter)?;

            chapter.content = content;
            icebergs.extend(new_icebergs);
        }
    }

    let target = context.root.join("src").join(".icebergs");
    fs::create_dir_all(&target)?;

    compiler.retain(&icebergs)?;
    compiler.release(&icebergs, target)?;

    Ok(book)
}

fn process_chapter(
    compiler: &Compiler,
    chapter: &Chapter,
) -> Result<(String, BTreeSet<compiler::Iceberg>), Error> {
    use itertools::Itertools;
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    use pulldown_cmark_to_cmark::cmark;

    let events = Parser::new_ext(&chapter.content, Options::all());

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

    let mut icebergs = BTreeSet::new();

    let output = groups.into_iter().flat_map(|(is_iced_code, group)| {
        if is_iced_code {
            let parts: Vec<_> = group.collect();

            if let Some(Event::Text(code)) = parts.get(1) {
                match compiler.compile(code) {
                    Ok(iceberg) => {
                        let embedding = iceberg.embed();
                        icebergs.insert(iceberg);

                        Box::new(
                            parts
                                .into_iter()
                                .chain(std::iter::once(Event::InlineHtml(embedding.into()))),
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

    Ok((content, icebergs))
}