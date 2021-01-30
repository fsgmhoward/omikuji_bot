#![feature(str_split_as_str)]
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

use diesel::mysql::MysqlConnection;
use diesel::prelude::*;
use rand::{thread_rng, Rng};
use std::env;
use telegram_bot::Error;
use telegram_bot::*;

pub mod models;
pub mod schema;

diesel_migrations::embed_migrations!();

pub fn establish_connection() -> MysqlConnection {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let connection = MysqlConnection::establish(&database_url)
        .expect(&format!("Error connecting to {}", database_url));
    embedded_migrations::run(&connection).expect("Failed to run migrations");
    println!("MySQL connection is established");
    connection
}

pub fn new_message(message: &str, from: &User, connection: &MysqlConnection) {
    let user_id = from.id.into();
    let mut user_name = from.first_name.clone();
    if let Some(last_name) = &from.last_name {
        user_name.push(' ');
        user_name.push_str(last_name.as_str());
    }
    let message = models::NewMessage {
        message: message,
        tg_id: user_id,
        tg_name: &user_name,
    };
    diesel::insert_into(schema::messages::table)
        .values(&message)
        .execute(connection)
        .expect("Failed to insert!");
}

// If upvote == true, ++vote_count, -- otherwise
pub fn vote(message: &models::Message, upvote: bool, connection: &MysqlConnection) {
    use schema::messages::dsl::vote_count;
    diesel::update(message)
        .set(vote_count.eq(message.vote_count + (if upvote { 1 } else { -1 })))
        .execute(connection)
        .expect(format!("Failed to update vote_count for message {:?}", message).as_str());
}

pub fn get_random_message(connection: &MysqlConnection) -> Option<models::Message> {
    use schema::messages::dsl::{id, messages, vote_count};
    let count: i64 = messages
        .filter(vote_count.gt(-3))
        .count()
        .get_result(connection)
        .expect("Unable to get row count");
    if count == 0 {
        return None;
    }
    let mut rng = thread_rng();
    let x: i64 = rng.gen_range(0, count - 1);
    Some(
        messages
            .filter(vote_count.gt(-3))
            .order(id)
            .limit(1)
            .offset(x)
            .get_result(connection)
            .expect(format!("Unable to retrieve row {}", x).as_str()),
    )
}

// Actual program logic for Omikuji bot
// Entry for all messages received
pub async fn message_entry(message: &Message, api: &Api) -> Result<(), Error> {
    if let MessageKind::Text { ref data, .. } = message.kind {
        // This is a text message
        if data.as_bytes()[0] == b'/' {
            // We consider all messages starting with '/' as a command
            command_entry(data, message, api).await?;
        } else {
            // Show them a welcome message
            api.send(SendMessage::new(
                &message.from,
                "Welcome to use NUSCAS's Omikuji Bot!\nTo start, simply type /start",
            ))
            .await?;
        }
    } else {
        api.send(SendMessage::new(
            &message.chat,
            "Sorry, non-text messages are not supported.",
        ))
        .await?;
    }
    Ok(())
}

// Entry for all callback received (from inline keyboard buttons)
pub async fn callback_entry(
    callback: &CallbackQuery,
    api: &Api,
    connection: &MysqlConnection,
) -> Result<(), Error> {
    if let Some(command) = &callback.data {
        // Try to split the command and the payload (metadata)
        let command_split: Vec<&str> = command.split('/').collect();
        let command = command_split[0];
        let payload = if command_split.len() > 1 {
            command_split[1]
        } else {
            ""
        };

        // We delete the original inline keyboard to prevent it being clicked for 2 times
        if let Some(original_message) = &callback.message {
            api.send(EditMessageReplyMarkup::new(
                &callback.from,
                original_message,
                None::<ReplyKeyboardMarkup>,
            ))
            .await?;
        }
        match command {
            // TODO
            "new" => draw(&callback.from, api, connection).await?,
            "draw" => draw(&callback.from, api, connection).await?,
            "vote" => {
                use schema::messages::dsl::{id, messages};
                if payload.len() <= 1 {
                    // Malformed payload - this should be +<id> or -<id>
                    api.send(SendMessage::new(
                        &callback.from,
                        "Malformed callback request.",
                    ))
                    .await?;
                    return Ok(());
                }
                let omikuji_id = &payload[1..payload.len()];
                if let Ok(omikuji_id) = omikuji_id.parse::<u32>() {
                    let omikuji = messages
                        .filter(id.eq(omikuji_id))
                        .limit(1)
                        .get_result(connection);
                    if let Ok(omikuji) = omikuji {
                        let is_upvote = payload.as_bytes()[0] == b'+';
                        vote(&omikuji, is_upvote, connection);
                        api.send(SendMessage::new(
                            &callback.from,
                            format!(
                                "Successfully {} the omikuji slip!",
                                if is_upvote { "upvoted" } else { "downvoted" }
                            ),
                        ))
                        .await?;
                    } else {
                        api.send(SendMessage::new(
                            &callback.from,
                            "Requested omikuji cannot be found.",
                        ))
                        .await?;
                    }
                } else {
                    api.send(SendMessage::new(
                        &callback.from,
                        "Malformed callback request.",
                    ))
                    .await?;
                }
            }
            _ => {
                api.send(SendMessage::new(
                    &callback.from,
                    "Callback query is not recognized!",
                ))
                .await?;
            }
        }
    } else {
        // This callback query contains empty query body - there must be something wrong
        api.send(SendMessage::new(
            &callback.from,
            "Callback query has empty body - probably your TG client is lousy!",
        ))
        .await?;
    }
    Ok(())
}

async fn command_entry(command: &String, message: &Message, api: &Api) -> Result<(), Error> {
    match command.as_str() {
        "/start" => welcome(message, api).await?,
        _ => {
            api.send(SendMessage::new(
                &message.from,
                format!("Command {} is not recognized.", command),
            ))
            .await?;
        }
    };
    Ok(())
}

// Action 0: Welcome a new user, and also reset previous keyboard
async fn welcome(message: &Message, api: &Api) -> Result<(), Error> {
    api.send(
        SendMessage::new(&message.chat, "Welcome to use NUSCAS's Omikuji Bot!")
            .reply_markup(reply_markup!(remove_keyboard)),
    )
    .await?;
    let keyboard = reply_markup!(inline_keyboard, [
        "Create new Omikuji" callback "new",
        "Draw an Omikuji slip" callback "draw"
    ]);
    api.send(SendMessage::new(&message.chat, "Pick what you want to do!").reply_markup(keyboard))
        .await?;
    Ok(())
}

// Action 1: Draw an omikuji
async fn draw(from: &User, api: &Api, connection: &MysqlConnection) -> Result<(), Error> {
    let random_message = get_random_message(connection);
    if let Some(random_message) = random_message {
        // only send if a message is available
        let keyboard = reply_markup!(inline_keyboard, [
            "This slip is well written" callback ("vote/+".to_owned() + &random_message.id.to_string()),
            "I feel insulted :(" callback ("vote/-".to_owned() + &random_message.id.to_string())
        ]);
        api.send(SendMessage::new(from, random_message.message).reply_markup(keyboard))
            .await?;
    } else {
        api.send(SendMessage::new(
            from,
            "Oops! Our omikuji library is empty.",
        ))
        .await?;
    }
    Ok(())
}
