extern crate chrono;
#[macro_use]
extern crate diesel;
extern crate egg_mode;
#[macro_use]
extern crate failure;
extern crate regex;
extern crate tokio_core;

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use egg_mode::{KeyPair, Token};
use egg_mode::tweet::{DraftTweet, Tweet};
use failure::Error;
use tokio_core::reactor::Core;

mod schema;
use schema::events;

#[derive(Debug, Insertable)]
#[table_name = "events"]
struct EventForm<'a> {
    tweet_id: i64,
    celestial_body: &'a str,
    replied: bool,
    deadline: NaiveDateTime,
    round_trip: f64,
}

#[derive(Debug, Queryable)]
struct Event {
    id: i32,
    tweet_id: i64,
    celestial_body: String,
    replies: bool,
    deadline: NaiveDateTime,
    created_at: NaiveDateTime,
    round_trip: f64,
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
    process_new_mentions(&conn, &token)?;
    send_replies(&conn, &token)?;
    Ok(())
}

fn process_new_mentions(conn: &SqliteConnection, token: &Token) -> Result<(), Error> {
    let mut core = Core::new()?;
    let handle = core.handle();
    let max_id = get_max_id(&conn)?;

    let mut mentions = egg_mode::tweet::mentions_timeline(&token, &handle);
    loop {
        let (new_mentions, feed) = core.run(mentions.older(max_id))?;
        mentions = new_mentions;
        if feed.is_empty() {
            return Ok(());
        }

        // @Todo: async fetching
        for tweet in &feed {
            match build_event(&tweet) {
                Ok(Response::RecordForm(event)) => {
                    if let Err(err) = diesel::insert_into(events::table)
                        .values(&event)
                        .execute(conn)
                    {
                        eprintln!("Error inserting tweet: {}", err);
                    }
                    println!("{:?}", event);
                }
                Ok(Response::Reply(draft)) => {
                    let draft = draft.auto_populate_reply_metadata(true).in_reply_to(tweet.id);
                    core.run(draft.send(&token, &handle))?;
                }
                Err(err) => eprintln!("Error processing tweet: {}", err),
            }
        }
    }
}

enum Response<'a> {
    RecordForm(EventForm<'a>),
    Reply(DraftTweet<'a>),
}

fn build_event(tweet: &Tweet) -> Result<Response, Error> {
    let mut body = tweet.text.trim();
    if body.starts_with("@celestial_echo") {
        body = &body["@celestial_echo".len()..].trim();
    }

    let out = std::process::Command::new("expect")
        .arg("horizons")
        .arg(&tweet.created_at.format("%Y-%m-%d %H:%M:%S").to_string())
        .arg(body)
        .output()?;

    match out.status.code() {
        Some(0) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text.trim().lines().next().ok_or_else(|| format_err!("HORIZON response missing distance line"))?;
            let dist = line.split_whitespace().nth(2).ok_or_else(|| format_err!("Missing distance field: '{}'", line))?;
            let travel_secs = dist.parse::<f64>()? * 60.0 * 2.0;
            let travel_time = chrono::Duration::milliseconds((travel_secs * 1000.0) as i64);
            let deadline = tweet.created_at + travel_time;
            return Ok(Response::RecordForm(EventForm {
                tweet_id: tweet.id as i64,
                celestial_body: body,
                replied: false,
                deadline: deadline.naive_utc(),
                round_trip: travel_secs,
            }))
        }
        Some(1) => {
            return Ok(Response::Reply(DraftTweet::new(r#"Sorry, I don't recognize that location.

Consult JPL HORIZONS for valid options: https://ssd.jpl.nasa.gov/?horizons
"#)));
        }
        Some(2) => {
            let pattern = regex::Regex::new(r#" *(-?\d+) *(.*?)(\(|  |$)"#).unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let lines = text.trim().lines();
            let mut message = String::from("Pick a number:\n");
            for line in lines {
                let cap = pattern.captures(line).ok_or_else(|| format_err!("No match found in '{}'", line))?;
                let line = format!("{}: {}\n", cap.get(1).unwrap().as_str(), cap.get(2).unwrap().as_str().trim());
                if message.len() + line.len() <= 280 {
                    message += &line;
                }
            }
            return Ok(Response::Reply(DraftTweet::new(message)));
        }
        code => {
            return Err(format_err!("Unrecognized exit code: {:?}", code));
        }
    }
}

fn send_replies(_conn: &SqliteConnection, _token: &Token) -> Result<(), Error> {
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
        },
    })
}

fn establish_connection() -> Result<SqliteConnection, Error> {
    let url = get_env("DATABASE_URL")?;
    Ok(SqliteConnection::establish(&url)?)
}

fn get_env(var: &str) -> Result<String, Error> {
    Ok(::std::env::var(var).map_err(|e| format_err!("{} '{}'", e, var))?)
}
