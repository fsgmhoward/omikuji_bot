#![feature(str_split_as_str)]
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

use async_trait::async_trait;
use diesel::mysql::MysqlConnection;
use diesel::prelude::*;
use rand::{thread_rng, Rng};
use std::env;
use telegram_bot::Error;
use telegram_bot::*;

pub mod models;
pub mod schema;

diesel_migrations::embed_migrations!();

//
// Extensions / Syntax Sugars
//

#[async_trait]
pub trait ApiExtension {
    async fn send_message(&self, to: &User, message: &str) -> Result<(), Error>;
    async fn send_photo(&self, to: &User, photo: &String) -> Result<(), Error>;
}

#[async_trait]
impl ApiExtension for Api {
    async fn send_message(&self, to: &User, message: &str) -> Result<(), Error> {
        self.send(SendMessage::new(to, message)).await?;
        Ok(())
    }
    async fn send_photo(&self, to: &User, photo: &String) -> Result<(), Error> {
        self.send(SendPhoto::new(to, FileRef::from(photo.clone())))
            .await?;
        Ok(())
    }
}

//
// Helper functions for database connection
//

pub fn establish_connection() -> MysqlConnection {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let connection = MysqlConnection::establish(&database_url)
        .expect(&format!("Error connecting to {}", database_url));
    embedded_migrations::run(&connection).expect("Failed to run migrations");
    println!("MySQL connection is established");
    connection
}

//
// Functions for manipulating omikuji records
//

pub fn new_omikuji(message: &str, from: &User, connection: &MysqlConnection) {
    let user_id = from.id.into();
    let mut user_name = from.first_name.clone();
    if let Some(last_name) = &from.last_name {
        user_name.push(' ');
        user_name.push_str(last_name.as_str());
    }
    let omikuji = models::NewOmikuji {
        message: message,
        tg_id: user_id,
        tg_name: &user_name,
    };
    diesel::insert_into(schema::omikujis::table)
        .values(&omikuji)
        .execute(connection)
        .expect("Failed to insert!");
}

// If upvote == true, ++vote_count, -- otherwise
pub fn vote(omikuji: &models::Omikuji, upvote: bool, connection: &MysqlConnection) {
    use schema::omikujis::dsl::vote_count;
    diesel::update(omikuji)
        .set(vote_count.eq(omikuji.vote_count + (if upvote { 1 } else { -1 })))
        .execute(connection)
        .expect(format!("Failed to update vote_count for omikuji {:?}", omikuji).as_str());
}

pub fn get_random_omikuji(connection: &MysqlConnection) -> Option<models::Omikuji> {
    use schema::omikujis::dsl::{id, omikujis, vote_count};
    let count: i64 = omikujis
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
        omikujis
            .filter(vote_count.gt(-3))
            .order(id)
            .limit(1)
            .offset(x)
            .get_result(connection)
            .expect(format!("Unable to retrieve row {}", x).as_str()),
    )
}

//
// Functions for handling client-side inputs
//

// Entry for all messages received
pub async fn message_entry(message: &Message, api: &Api) -> Result<(), Error> {
    let from = &message.from;
    match message.kind {
        MessageKind::Text { ref data, .. } => {
            // This is a text message
            if data.as_bytes()[0] == b'/' {
                // We consider all messages starting with '/' as a command
                match data.as_str() {
                    "/start" => welcome(message, api).await?,
                    _ => {
                        api.send_message(
                            from,
                            format!("Command {} is not recognized.", data).as_str(),
                        )
                        .await?;
                    }
                };
                return Ok(());
            }

            // Show them a welcome message for any text input
            api.send_message(
                from,
                "Welcome to use NUSCAS's Omikuji Bot!\nTo start, simply type /start",
            )
            .await?;
        }
        MessageKind::Photo { ref data, .. } => {
            if data.len() == 0 {
                api.send_message(from, "Malformed image").await?;
                return Ok(());
            }
            let photo = &data[0].file_id;
            // TODO
            api.send_message(from, format!("Image received. ID = {}", photo).as_str())
                .await?;
            api.send_photo(from, photo).await?;
        }
        _ => {
            api.send_message(from, "Sorry, this kind of message is yet to be supported.")
                .await?;
        }
    }
    Ok(())
}

// Entry for all callback received (from inline keyboard buttons)
pub async fn callback_entry(
    callback: &CallbackQuery,
    api: &Api,
    connection: &MysqlConnection,
) -> Result<(), Error> {
    let from = &callback.from;
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
        if let Some(message) = &callback.message {
            api.send(EditMessageReplyMarkup::new(
                from,
                message,
                None::<ReplyKeyboardMarkup>,
            ))
            .await?;
        }
        match command {
            // TODO
            "new" => draw(from, api, connection).await?,
            "draw" => draw(from, api, connection).await?,
            "vote" => {
                use schema::omikujis::dsl::{id, omikujis};
                if payload.len() <= 1 {
                    // Malformed payload - this should be +<id> or -<id>
                    api.send_message(from, "Malformed callback request.")
                        .await?;
                    return Ok(());
                }
                let omikuji_id = &payload[1..payload.len()];
                if let Ok(omikuji_id) = omikuji_id.parse::<u32>() {
                    let omikuji = omikujis
                        .filter(id.eq(omikuji_id))
                        .limit(1)
                        .get_result(connection);
                    if let Ok(omikuji) = omikuji {
                        let is_upvote = payload.as_bytes()[0] == b'+';
                        vote(&omikuji, is_upvote, connection);
                        api.send_message(
                            from,
                            format!(
                                "Successfully {} the omikuji slip!",
                                if is_upvote { "upvoted" } else { "downvoted" }
                            )
                            .as_str(),
                        )
                        .await?;
                    } else {
                        api.send_message(from, "Requested omikuji cannot be found.")
                            .await?;
                    }
                } else {
                    api.send_message(from, "Malformed callback request.")
                        .await?;
                }
            }
            _ => {
                api.send_message(from, "Callback query is not recognized!")
                    .await?;
            }
        }
    } else {
        // This callback query contains empty query body - there must be something wrong
        api.send_message(
            from,
            "Callback query has empty body - probably your TG client is lousy!",
        )
        .await?;
    }
    Ok(())
}

//
// Functions for different actions
//

// Action 0: Welcome a new user, and also reset previous keyboard
async fn welcome(message: &Message, api: &Api) -> Result<(), Error> {
    let chat = &message.chat;
    api.send(
        SendMessage::new(chat, "Welcome to use NUSCAS's Omikuji Bot!")
            .reply_markup(reply_markup!(remove_keyboard)),
    )
    .await?;
    let keyboard = reply_markup!(inline_keyboard, [
        "Create new Omikuji" callback "new",
        "Draw an Omikuji slip" callback "draw"
    ]);
    api.send(SendMessage::new(chat, "Pick what you want to do!").reply_markup(keyboard))
        .await?;
    Ok(())
}

// Action 1: Draw an omikuji
async fn draw(from: &User, api: &Api, connection: &MysqlConnection) -> Result<(), Error> {
    let omikuji = get_random_omikuji(connection);
    if let Some(omikuji) = omikuji {
        // only send if a message is available
        let keyboard = reply_markup!(inline_keyboard, [
            "This slip is well written" callback ("vote/+".to_owned() + &omikuji.id.to_string()),
            "I feel insulted :(" callback ("vote/-".to_owned() + &omikuji.id.to_string())
        ]);
        api.send(SendMessage::new(from, omikuji.message).reply_markup(keyboard))
            .await?;
    } else {
        api.send_message(from, "Oops! Our omikuji library is empty.")
            .await?;
    }
    Ok(())
}
