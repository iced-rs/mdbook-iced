use mdbook_iced::{is_supported, run};

use clap::{Arg, Command};
use mdbook::errors::Error;
use mdbook::preprocess::CmdPreprocessor;

use std::io;
use std::process;

fn main() -> Result<(), Error> {
    let command = Command::new("mdbook-iced")
        .about("An mdBook preprocessor to turn iced code blocks into interactive examples.")
        .subcommand(
            Command::new("supports")
                .arg(Arg::new("renderer").required(true))
                .about("Check whether a renderer is supported by this preprocessor"),
        );

    let matches = command.get_matches();

    if let Some(sub_args) = matches.subcommand_matches("supports") {
        let renderer = sub_args
            .get_one::<String>("renderer")
            .expect("Required argument");

        if !is_supported(renderer) {
            process::exit(1);
        }

        process::exit(0);
    }

    let (context, book) = CmdPreprocessor::parse_input(io::stdin())?;

    let processed_book = run(book, &context)?;
    serde_json::to_writer(io::stdout(), &processed_book)?;

    Ok(())
}
