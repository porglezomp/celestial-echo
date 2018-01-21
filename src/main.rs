extern crate chrono;
#[macro_use]
extern crate diesel;
extern crate egg_mode;
#[macro_use]
extern crate failure;
extern crate regex;
extern crate tokio_core;

use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use egg_mode::{KeyPair, Token};
use egg_mode::tweet::{DraftTweet, Tweet};
use failure::Error;
use tokio_core::reactor::Core;

mod schema;
use schema::{events, ignored};

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
    round_trip: f64,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

#[derive(Insertable)]
#[table_name = "ignored"]
struct Ignored {
    tweet_id: i64,
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
                    if is_tweet_ignored(conn, tweet.id).unwrap_or(true) {
                        continue;
                    }
                    let draft = draft
                        .auto_populate_reply_metadata(true)
                        .in_reply_to(tweet.id);
                    match core.run(draft.send(&token, &handle)) {
                        Ok(_) => match ignore_tweet(conn, tweet.id) {
                            Ok(()) => (),
                            Err(err) => eprintln!("Error ignoring tweet {}: {}", tweet.id, err),
                        },
                        Err(err) => eprintln!("Error replying to tweet: {}", err),
                    }
                }
                Err(err) => eprintln!("Error processing tweet: {}", err),
            }
        }
    }
}

fn is_tweet_ignored(conn: &SqliteConnection, t_id: u64) -> Result<bool, Error> {
    use schema::ignored::dsl::*;
    Ok(!ignored
        .filter(tweet_id.eq(t_id as i64))
        .limit(1)
        .load::<(i32, i64)>(conn)?
        .is_empty())
}

fn ignore_tweet(conn: &SqliteConnection, t_id: u64) -> Result<(), Error> {
    use schema::ignored::dsl::*;
    diesel::insert_into(ignored)
        .values(&Ignored {
            tweet_id: t_id as i64,
        })
        .execute(conn)?;
    Ok(())
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
            let line = text.trim()
                .lines()
                .next()
                .ok_or_else(|| format_err!("HORIZON response missing distance line"))?;
            let dist = line.split_whitespace()
                .nth(2)
                .ok_or_else(|| format_err!("Missing distance field: '{}'", line))?;
            println!("{}", line);
            let travel_secs = dist.parse::<f64>()? * 60.0 * 2.0;
            let travel_time = chrono::Duration::milliseconds((travel_secs * 1000.0) as i64);
            let deadline = tweet.created_at + travel_time;
            return Ok(Response::RecordForm(EventForm {
                tweet_id: tweet.id as i64,
                celestial_body: body,
                replied: false,
                deadline: deadline.naive_utc(),
                round_trip: travel_secs,
            }));
        }
        Some(1) => {
            return Ok(Response::Reply(DraftTweet::new(
                r#"Sorry, I don't recognize that location.

Consult JPL HORIZONS for valid options: https://ssd.jpl.nasa.gov/?horizons
"#,
            )));
        }
        Some(2) => {
            let pattern = regex::Regex::new(r#" *(-?\d+) *(.*?)(\(|  |$)"#).unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let lines = text.trim().lines();
            let mut message = String::from("Pick a number:\n");
            for line in lines {
                let cap = pattern
                    .captures(line)
                    .ok_or_else(|| format_err!("No match found in '{}'", line))?;
                let line = format!(
                    "{}: {}\n",
                    cap.get(1).unwrap().as_str(),
                    cap.get(2).unwrap().as_str().trim()
                );
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

fn send_replies(conn: &SqliteConnection, token: &Token) -> Result<(), Error> {
    use schema::events::dsl::*;

    let mut core = Core::new()?;
    let handle = core.handle();

    let unreplied = events
        .filter(replied.eq(false).and(deadline.lt(Utc::now().naive_utc())))
        .load::<Event>(conn)?;
    for event in unreplied {
        // Different precisions: 1.0035s, 8.325s, 48.53s, 1m 3.2s, 13h 2m
        let hr = event.round_trip as u32 / 3600;
        let min = (event.round_trip as u32 / 60) % 60;
        let sec = event.round_trip % 60.0;
        let msg = if event.round_trip < 2.0 {
            format!("Round trip time: {:.4}s", event.round_trip)
        } else if event.round_trip < 10.0 {
            format!("Round trip time: {:.3}s", event.round_trip)
        } else if event.round_trip < 60.0 {
            format!("Round trip time: {:.2}s", event.round_trip)
        } else if min < 10 {
            format!("Round trip time: {}m {:.1}s", min, sec)
        } else if min < 60 {
            format!("Round trip time: {}m {}s", min, sec as u32)
        } else {
            format!("Round trip time: {}h {}m", hr, min)
        };

        let reply = DraftTweet::new(msg)
            .auto_populate_reply_metadata(true)
            .in_reply_to(event.tweet_id as u64);
        match core.run(reply.send(&token, &handle)) {
            Ok(_) => {
                diesel::update(events.filter(id.eq(event.id)))
                    .set(replied.eq(true))
                    .execute(conn)?;
            }
            Err(err) => eprintln!("Error replying to {}: {}", event.tweet_id, err),
        }
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
