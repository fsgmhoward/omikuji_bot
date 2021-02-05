#![feature(str_split_as_str)]
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

use anyhow::Error;
use async_trait::async_trait;
use diesel::mysql::MysqlConnection;
use diesel::prelude::*;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::str::FromStr;
use strum::IntoEnumIterator;
use telegram_bot::*;

pub mod models;
pub mod schema;

use models::OmikujiClass;
use models::OmikujiMessage;
use models::OmikujiSection;

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
        self.send(SendMessage::new(to, message).parse_mode(ParseMode::Markdown))
            .await?;
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
    fn delete_user_data(&mut self, user: &User);
}

impl HashMapExtension for HashMap<i64, OmikujiMessage> {
    fn get_user_data(&mut self, user: &User) -> Option<&mut OmikujiMessage> {
        self.get_mut(&i64::from(user.id))
    }

    fn new_user_data(&mut self, user: &User) {
        let omikuji_message = OmikujiMessage {
            photo: None,
            class: None,
            description: None,
            sections: Vec::new(),
        };
        self.insert(i64::from(user.id), omikuji_message);
    }

    fn delete_user_data(&mut self, user: &User) {
        self.remove(&i64::from(user.id));
    }
}

trait EnumExtension: IntoEnumIterator + fmt::Debug {
    fn to_keyboard(callback_command: &str) -> InlineKeyboardMarkup {
        let mut keyboard = InlineKeyboardMarkup::new();
        let mut sections = Vec::<String>::new();
        // TODO
        let per_row = 2;
        for section in Self::iter() {
            sections.push(format!("{:?}", section));
        }
        for i in (0..sections.len()).step_by(per_row) {
            let mut buttons = Vec::<InlineKeyboardButton>::new();
            for j in 0..per_row {
                let index = i + j;
                if index >= sections.len() {
                    break;
                }
                let section = &sections[index];
                buttons.push(InlineKeyboardButton::callback(
                    section,
                    format!("{}/{}", callback_command, section),
                ));
            }
            keyboard.add_row(buttons);
        }
        return keyboard;
    }
}

impl EnumExtension for OmikujiClass {}
impl EnumExtension for OmikujiSection {}

impl fmt::Display for OmikujiMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        if let Some(class) = &self.class {
            text += format!("*{:?}*\n", class).as_str();
        }
        if let Some(description) = &self.description {
            text += format!("{}\n", description).as_str();
        }
        for (section_name, description) in &self.sections {
            text += format!("\n*{:?}*: {}", section_name, description).as_str();
        }
        write!(f, "{}", text)
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

fn new_omikuji(message: &str, from: &User, connection: &MysqlConnection) {
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

fn get_random_omikuji(connection: &MysqlConnection) -> Option<models::Omikuji> {
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
    // Note: gen_range generates a number in range [low, high) so low < high
    let x: i64 = rng.gen_range(0, count);
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
    connection: &MysqlConnection,
) -> Result<(), Error> {
    let from = &message.from;
    match message.kind {
        MessageKind::Text { ref data, .. } => {
            // This is a text message
            if data.as_bytes()[0] == b'/' {
                // We consider all messages starting with '/' as a command
                match data.as_str() {
                    "/start" => start(from, api).await?,
                    "/current" => current(from, api, store).await?,
                    "/cancel" => cancel(from, api, store).await?,
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

            if update_description(from, api, store, data).await? {
                // This message has been captured as a description, so don't do anything else
                return Ok(());
            }

            if !update_section(from, api, store, data).await? {
                // Show user a welcome message for text input if no section has been updated
                api.send_message(
                    from,
                    "Welcome to use NUSCAS's Omikuji Bot!\nTo start, simply type /start",
                )
                .await?;
            }
        }
        MessageKind::Photo { ref data, .. } => {
            if data.len() == 0 {
                api.send_message(from, "Malformed image").await?;
                return Ok(());
            }
            let photo = &data[0].file_id;
            save(from, api, store, connection, Some(photo.to_string())).await?;
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
        // We will ignore the error generated here
        if let Some(message) = &callback.message {
            #[allow(unused_must_use)]
            {
                api.send(EditMessageReplyMarkup::new(
                    from,
                    message,
                    None::<ReplyKeyboardMarkup>,
                ))
                .await;
            }
        }
        match command {
            // Sequence: from, api, store, connection, payload/photo
            "new" => new(from, api, store).await?,
            "draw" => draw(from, api, connection).await?,
            "class" => class(from, api, store, payload).await?,
            "section" => section(from, api, store, payload).await?,
            "ask_photo" => ask_photo(from, api).await?,
            "save" => save(from, api, store, connection, None).await?,
            "vote" => vote(from, api, connection, payload).await?,
            _ => {
                api.send_message(
                    from,
                    format!("Callback query {} is not recognized!", command).as_str(),
                )
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

// Welcome a new user, and also reset previous keyboard
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

async fn current(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
) -> Result<(), Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        api.send_message(
            from,
            format!(
                "This is what you are currently working on:\n\n{}",
                omikuji_message
            )
            .as_str(),
        )
        .await?;
    } else {
        api.send_message(
            from,
            "You don't have an omikuji you are currently working on.",
        )
        .await?;
    }
    Ok(())
}

async fn cancel(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
) -> Result<(), Error> {
    store.delete_user_data(from);
    api.send_message(from, "Fine. I have delete current work-in-progress omikuji. You can start a new one by calling /start !").await?;
    Ok(())
}

async fn about(from: &User, api: &Api) -> Result<(), Error> {
    api.send_message(
        from,
        "This is a bot used for storing and drawing Omikuji strips, written by @FSGMHoward.\n\
        Source code can be found on https://github.com/fsgmhoward/omikuji_bot",
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

// Check if the user need to update the description
async fn update_description(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
    payload: &str,
) -> Result<bool, Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        if let None = omikuji_message.description {
            omikuji_message.description = Some(String::from(payload));
            let keyboard = OmikujiSection::to_keyboard("section");
            api.send(
                SendMessage::new(from, "Nice. Now, select the first section below.")
                    .reply_markup(ReplyMarkup::InlineKeyboardMarkup(keyboard)),
            )
            .await?;
            return Ok(true);
        }
    }
    return Ok(false);
}

// Check if the user has a pending omikuji which is yet to be submitted
// Return Ok(true) if an omikuji strip is updated or anything wrong occurred
async fn update_section(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
    payload: &str,
) -> Result<bool, Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        // Determine which part this message is updating
        let section_count = omikuji_message.sections.len();
        if section_count == 0 {
            api.send_message(
                from,
                "You will need to select a section type before entering any description!",
            )
            .await?;
            return Ok(true);
        }
        let (_, description) = &mut omikuji_message.sections[section_count - 1];
        if description != "" {
            // We don't modify a section if it already has description
            api.send_message(
                from,
                "You will need to select a section type before entering any description!",
            )
            .await?;
            return Ok(true);
        }
        description.push_str(payload);
        let mut keyboard = OmikujiSection::to_keyboard("section");
        keyboard.add_row(vec![InlineKeyboardButton::callback(
            "Just save what is done!",
            "ask_photo",
        )]);
        api.send(
            SendMessage::new(from, "Sure. Do you want to add a new section or just save?")
                .reply_markup(ReplyMarkup::InlineKeyboardMarkup(keyboard)),
        )
        .await?;

        return Ok(true);
    }
    return Ok(false);
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

    let keyboard = OmikujiClass::to_keyboard("class");

    api.send(
        SendMessage::new(from, "Ok. Select a class from below!")
            .reply_markup(ReplyMarkup::InlineKeyboardMarkup(keyboard)),
    )
    .await?;
    Ok(())
}

// Draw an omikuji
async fn draw(from: &User, api: &Api, connection: &MysqlConnection) -> Result<(), Error> {
    let omikuji = get_random_omikuji(connection);
    if let Some(omikuji) = omikuji {
        let omikuji_message: OmikujiMessage = serde_json::from_str(omikuji.message.as_str())?;
        if let Some(photo) = &omikuji_message.photo {
            api.send_photo(from, photo).await?;
        }

        let mut text = String::from("You draw a omikuji strip:\n\n");
        text += format!("{}", omikuji_message).as_str();

        // only send if a message is available
        let keyboard = reply_markup!(inline_keyboard, [
            "This slip is well written" callback (format!("vote/+{}", omikuji.id.to_string())),
            "I feel insulted :(" callback (format!("vote/-{}", omikuji.id.to_string()))
        ]);
        api.send(
            SendMessage::new(from, text)
                .parse_mode(ParseMode::Markdown)
                .reply_markup(keyboard),
        )
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
        if let Some(_) = omikuji_message.class {
            api.send_message(from, "You have already set the class of this strip.")
                .await?;
            return Ok(());
        }
        if let Ok(class) = OmikujiClass::from_str(payload) {
            api.send_message(from, "Sure! Can you write a brief description for it?")
                .await?;
            if let OmikujiClass::Other = class {
                api.send_message(from, "Since you choose `Other` for the class, probably you want to name your class in the description as well?").await?;
            }
            omikuji_message.class = Some(class);
        } else {
            api.send_message(from, "Malformed callback request.")
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

async fn section(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
    payload: &str,
) -> Result<(), Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        if let None = omikuji_message.class {
            api.send_message(
                from,
                "You have to choose a class before create a new section!",
            )
            .await?;
            return Ok(());
        }
        if let None = omikuji_message.description {
            api.send_message(
                from,
                "You have to enter brief description before create a new section!",
            )
            .await?;
            return Ok(());
        }
        let section_count = omikuji_message.sections.len();
        if section_count != 0 {
            let (_, description) = &omikuji_message.sections[section_count - 1];
            if description == "" {
                api.send_message(
                    from,
                    "You have to type the description for the previous section first",
                )
                .await?;
                return Ok(());
            }
        }

        if let Ok(section) = OmikujiSection::from_str(payload) {
            let reply = format!(
                "OK. Type your description for section {:?} below!",
                &section
            );
            omikuji_message.sections.push((section, String::new()));
            api.send_message(from, reply.as_str()).await?;
        } else {
            api.send_message(from, "Malformed callback request.")
                .await?;
        }
    } else {
        api.send_message(
            from,
            "You have to create a new omikuji strip before calling `section` callback.",
        )
        .await?;
    }
    Ok(())
}

async fn ask_photo(from: &User, api: &Api) -> Result<(), Error> {
    let keyboard = reply_markup!(inline_keyboard, [
        "No, just save it!" callback "save"
    ]);
    api.send(SendMessage::new(from, "Do you want to upload an image of your omikuji strip? Just send me a photo if you want to!").reply_markup(keyboard)).await?;
    Ok(())
}

async fn save(
    from: &User,
    api: &Api,
    store: &mut HashMap<i64, OmikujiMessage>,
    connection: &MysqlConnection,
    photo: Option<String>,
) -> Result<(), Error> {
    if let Some(omikuji_message) = store.get_user_data(from) {
        let section_count = omikuji_message.sections.len();
        if section_count != 0 {
            let (_, description) = &omikuji_message.sections[section_count - 1];
            // Check whether last section's description is filled in
            if description != "" {
                omikuji_message.photo = photo;
                let j = serde_json::to_string(omikuji_message)?;
                new_omikuji(j.as_str(), from, connection);
                store.delete_user_data(from);
                api.send_message(
                    from,
                    "Nice! Your omikuji strip has been saved into our database.",
                )
                .await?;
                return Ok(());
            }
        }
    }
    api.send_message(
        from,
        "You have to have a complete omikuji strip before executing `save`.",
    )
    .await?;
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
