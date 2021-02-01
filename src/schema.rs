table! {
    omikujis (id) {
        id -> Unsigned<Integer>,
        photo -> Nullable<Varchar>,
        message -> Mediumtext,
        vote_count -> Integer,
        tg_id -> Bigint,
        tg_name -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}
