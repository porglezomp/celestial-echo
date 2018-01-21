table! {
    events (id) {
        id -> Integer,
        tweet_id -> BigInt,
        celestial_body -> Text,
        replied -> Bool,
        deadline -> Timestamp,
        round_trip -> Double,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}
