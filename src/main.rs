//! `bdexmlconf` — convert Borland BDE configuration files to and from XML.

mod atomic;
mod bdeconf;
mod bdeconf_reader;
mod bdeconf_writer;
mod cli;
mod encoding;
mod merge;
mod paths;
mod xmlconf;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
