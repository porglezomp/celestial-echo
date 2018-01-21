CREATE TABLE events
  ( id INTEGER PRIMARY KEY NOT NULL
  , tweet_id UNSIGNED BIG INT NOT NULL
  , celestial_body TEXT NOT NULL
  , replied BOOLEAN NOT NULL DEFAULT 0
  , deadline TIMESTAMP NOT NULL
  , created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
  , updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
  );
