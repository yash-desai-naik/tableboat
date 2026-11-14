//! `tb`: Tableboat on the command line. A small console program next to the app: it reads the
//! same saved connections, passwords (the OS keychain), settings and Claude
//! projects, and works whether or not the app is open.

mod ask;
mod cli;
mod cmds;
mod out;
mod term;

use clap::Parser;
use cli::{Cli, Command, ConnCmd};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    term::init(cli.no_color);
    match run(cli.command).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e}", term::red("tb:"));
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> Result<ExitCode, String> {
    let ctx = cmds::Ctx::load()?;
    let code = match command {
        Command::Connections(a) => cmds::conn_list(&ctx, a.format)?,
        Command::Conn(c) => match c {
            ConnCmd::List(a) => cmds::conn_list(&ctx, a.format)?,
            ConnCmd::Show { connection } => cmds::conn_show(&ctx, &connection)?,
            ConnCmd::Add(a) => cmds::conn_add(&ctx, a).await?,
            ConnCmd::Edit(a) => cmds::conn_edit(&ctx, a)?,
            ConnCmd::Rm { connection, yes } => cmds::conn_rm(&ctx, &connection, yes)?,
            ConnCmd::Test { target } => cmds::conn_test(&ctx, &target).await?,
            ConnCmd::Url { connection, with_passwords } => cmds::conn_url(&ctx, &connection, with_passwords)?,
            ConnCmd::Duplicate { connection } => cmds::conn_duplicate(&ctx, &connection)?,
            ConnCmd::Export { file, with_passwords } => cmds::conn_export(&ctx, &file, with_passwords)?,
            ConnCmd::Import { file } => cmds::conn_import(&ctx, &file)?,
        },
        Command::Databases { connection, format } => cmds::databases(&ctx, &connection, format).await?,
        Command::Tables(a) => cmds::tables(&ctx, a).await?,
        Command::Describe(a) => cmds::describe(&ctx, a).await?,
        Command::Query(a) => cmds::query(&ctx, a).await?,
        Command::Export(a) => cmds::export(&ctx, a).await?,
        Command::Dump(a) => cmds::dump(&ctx, a).await?,
        Command::Ask(a) => ask::ask(&ctx, a).await?,
        Command::Open { connection } => cmds::open(&ctx, connection.as_deref())?,
        Command::Settings { cmd } => cmds::settings(cmd)?,
        Command::Version => cmds::version()?,
    };
    ctx.close().await;
    Ok(code)
}
