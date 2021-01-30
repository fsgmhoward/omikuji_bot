table! {
    messages (id) {
        id -> Unsigned<Integer>,
        message -> Varchar,
        vote_count -> Integer,
        tg_id -> Bigint,
        tg_name -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}
