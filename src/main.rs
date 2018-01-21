extern crate chrono;
extern crate diesel;
extern crate egg_mode;
#[macro_use]
extern crate failure;
extern crate tokio_core;

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use egg_mode::{Token, KeyPair};
use failure::Error;


fn main() {
    match run() {
        Ok(()) => (),
        Err(err) => {
            eprintln!("{}", err);
            ::std::process::exit(1);
        }
    }
}


fn run() -> Result<(), Error> {
    let conn = establish_connection()?;
    let token = auth()?;
    let mut core = tokio_core::reactor::Core::new()?;
    let handle = core.handle();

    let mentions = egg_mode::tweet::mentions_timeline(&token, &handle)
        .with_page_size(200);
    let (mentions, feed) = core.run(mentions.start())?;
    for tweet in &feed {
        let username = match tweet.user {
            Some(ref user) => &user.screen_name[..],
            None => "",
        };
        let mut text = tweet.text.trim();
        if text.starts_with("@celestial_echo") {
            text = &text["@celestial_echo".len()..].trim();
        }
        println!("{} @{}: {}", tweet.id, username, text);
    }

    Ok(())
}

fn auth() -> Result<Token, Error> {
    Ok(Token::Access {
        consumer: KeyPair {
            key: get_env("CONSUMER_KEY")?.into(),
            secret: get_env("CONSUMER_SECRET")?.into(),
        },
        access: KeyPair {
            key: get_env("ACCESS_KEY")?.into(),
            secret: get_env("ACCESS_SECRET")?.into(),
        }
    })
}

fn establish_connection() -> Result<SqliteConnection, Error> {
    let url = get_env("DATABASE_URL")?;
    Ok(SqliteConnection::establish(&url)?)
}

fn get_env(var: &str) -> Result<String, Error> {
    Ok(::std::env::var(var).map_err(|e| format_err!("{} '{}'", e, var))?)
}
