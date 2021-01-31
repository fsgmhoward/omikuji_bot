#![feature(str_split_as_str)]
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

use async_trait::async_trait;
use diesel::mysql::MysqlConnection;
use diesel::prelude::*;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use strum::IntoEnumIterator;
use telegram_bot::Error;
use telegram_bot::*;

pub mod models;
pub mod schema;

use models::OmikujiClass;
use models::OmikujiMessage;

diesel_migrations::embed_migrations!();

//
// Extensions / Syntax Sugars
//

#[async_trait]
trait ApiExtension {
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

trait HashMapExtension {
    fn get_user_data(&mut self, user: &User) -> Option<&mut OmikujiMessage>;
    fn new_user_data(&mut self, user: &User);
}

impl HashMapExtension for HashMap<i64, OmikujiMessage> {
    fn get_user_data(&mut self, user: &User) -> Option<&mut OmikujiMessage> {
        self.get_mut(&i64::from(user.id))
    }

    fn new_user_data(&mut self, user: &User) {
        let omikuji_message = OmikujiMessage {
            class: OmikujiClass::Unknown,
            sections: Vec::new(),
        };
        self.insert(i64::from(user.id), omikuji_message);
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
pub async fn message_entry(
    message: &Message,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
) -> Result<(), Error> {
    let from = &message.from;
    match message.kind {
        MessageKind::Text { ref data, .. } => {
            // This is a text message
            if data.as_bytes()[0] == b'/' {
                // We consider all messages starting with '/' as a command
                match data.as_str() {
                    "/start" => start(from, api).await?,
                    "/about" => about(from, api).await?,
                    "/debug" => debug(from, api, store).await?,
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

            // Check if the user has a pending omikuji which is yet to be submitted
            if let Some(omikuji_message) = store.get_user_data(from) {
                // Determine which part this message is updating
                let section_count = omikuji_message.sections.len();
                if section_count == 0 {
                    api.send_message(
                        from,
                        "You will need to select a section type before entering any description!",
                    )
                    .await?;
                    return Ok(());
                }
                let (_, description) = &mut omikuji_message.sections[section_count - 1];
                if description != "" {
                    // We don't modify a section if it already has description
                    api.send_message(
                        from,
                        "You will need to select a section type before entering any description!",
                    )
                    .await?;
                    return Ok(());
                }
                description.clear();
                description.push_str(data.as_str());
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
    store: &mut HashMap<i64, OmikujiMessage>,
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
            "new" => new(from, api, store).await?,
            "draw" => draw(from, api, connection).await?,
            "class" => class(from, api, store, payload).await?,
            "vote" => vote(from, api, connection, payload).await?,
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
async fn start(from: &User, api: &Api) -> Result<(), Error> {
    api.send(
        SendMessage::new(from, "Welcome to use NUSCAS's Omikuji Bot!")
            .reply_markup(reply_markup!(remove_keyboard)),
    )
    .await?;
    let keyboard = reply_markup!(inline_keyboard, [
        "Create new Omikuji" callback "new",
        "Draw an Omikuji slip" callback "draw"
    ]);
    api.send(SendMessage::new(from, "Pick what you want to do!").reply_markup(keyboard))
        .await?;
    Ok(())
}

async fn about(from: &User, api: &Api) -> Result<(), Error> {
    api.send_message(
        from,
        "This is a bot used for storing and drawing Omikuji strips, written by @FSGMHoward.",
    )
    .await?;
    Ok(())
}

// Print out the current strip
async fn debug(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
) -> Result<(), Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        api.send_message(from, format!("{:?}", omikuji_message).as_str())
            .await?;
    } else {
        api.send_message(from, "No omikuji strip stored.").await?;
    }
    Ok(())
}

async fn new(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
) -> Result<(), Error> {
    if let Some(_) = store.get_user_data(from) {
        api.send_message(
            from,
            "You have to complete your previous strip before creating a new one.",
        )
        .await?;
        return Ok(());
    }
    store.new_user_data(from);

    let mut keyboard = InlineKeyboardMarkup::new();
    for class in OmikujiClass::iter() {
        let class = format!("{:?}", class);
        let button = InlineKeyboardButton::callback(&class, format!("class/{}", class));
        keyboard.add_row(vec![button]);
    }

    api.send(
        SendMessage::new(from, "Ok. Select a class from below!")
            .reply_markup(ReplyMarkup::InlineKeyboardMarkup(keyboard)),
    )
    .await?;
    Ok(())
}

// Action 1: Draw an omikuji
async fn draw(from: &User, api: &Api, connection: &MysqlConnection) -> Result<(), Error> {
    let omikuji = get_random_omikuji(connection);
    if let Some(omikuji) = omikuji {
        // only send if a message is available
        let keyboard = reply_markup!(inline_keyboard, [
            "This slip is well written" callback (format!("vote/+{}", omikuji.id.to_string())),
            "I feel insulted :(" callback (format!("vote/-{}", omikuji.id.to_string()))
        ]);
        api.send(SendMessage::new(from, omikuji.message).reply_markup(keyboard))
            .await?;
    } else {
        api.send_message(from, "Oops! Our omikuji library is empty.")
            .await?;
    }
    Ok(())
}

// Update the class of the omikuji strip
async fn class(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
    payload: &str,
) -> Result<(), Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        if let OmikujiClass::Unknown = omikuji_message.class {
            if let Ok(class) = OmikujiClass::from_str(payload) {
                omikuji_message.class = class;
                // TODO let them add a section now
            } else {
                api.send_message(from, "Malformed callback request.")
                    .await?;
            }
        } else {
            api.send_message(from, "You have already set the class of this strip.")
                .await?;
        }
    } else {
        api.send_message(
            from,
            "You have to create a new omikuji strip before calling `class` callback.",
        )
        .await?;
    }
    Ok(())
}

async fn vote(
    from: &User,
    api: &Api,
    connection: &MysqlConnection,
    payload: &str,
) -> Result<(), Error> {
    use schema::omikujis::dsl::{id, omikujis, vote_count};
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
            diesel::update(&omikuji)
                .set(vote_count.eq(&omikuji.vote_count + (if is_upvote { 1 } else { -1 })))
                .execute(connection)
                .expect(format!("Failed to update vote_count for omikuji {:?}", &omikuji).as_str());
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
    Ok(())
}
