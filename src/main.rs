extern crate chrono;
#[macro_use]
extern crate diesel;
extern crate egg_mode;
#[macro_use]
extern crate failure;
extern crate tokio_core;

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use egg_mode::{Token, KeyPair};
use failure::Error;

mod schema;
use schema::events;

#[derive(Insertable)]
#[table_name = "events"]
struct EventForm<'a> {
    tweet_id: i64,
    celestial_body: &'a str,
    replied: bool,
    deadline: NaiveDateTime,
}

#[derive(Queryable)]
struct Event {
    id: i32,
    tweet_id: i64,
    celestial_body: String,
    replies: bool,
    deadline: NaiveDateTime,
    created_at: NaiveDateTime,
}


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

    let max_id = get_max_id(&conn)?;
    println!("Max id: {:?}", max_id);

    let mentions = egg_mode::tweet::mentions_timeline(&token, &handle)
        .with_page_size(200);
    let (mentions, feed) = core.run(mentions.older(max_id))?;
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
        // use schema::events::dsl::*;
        diesel::insert_into(events::table)
            .values(&EventForm {
                tweet_id: tweet.id as i64,
                celestial_body: text,
                replied: false,
                deadline: NaiveDateTime::from_timestamp(1000, 1000),
            })
            .execute(&conn)?;
    }

    Ok(())
}

fn get_max_id(conn: &SqliteConnection) -> Result<Option<u64>, Error> {
    use diesel::dsl::max;
    use schema::events::dsl::*;
    let max_id: Option<i64> = events.select(max(tweet_id)).first(conn)?;
    Ok(max_id.map(|x| x as u64))
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
