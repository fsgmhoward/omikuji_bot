use anyhow::Error;
use dotenv::dotenv;
use futures::StreamExt;
use models::OmikujiMessage;
use omikuji_bot::*;
use std::collections::HashMap;
use std::env;
use telegram_bot::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv().ok();

    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let api = Api::new(token);

    let mut store = HashMap::<i64, OmikujiMessage>::new();

    // Establish a connection to database server
    let connection = establish_connection();

    // Fetch new updates via long poll method
    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        let update = update?;
        match update.kind {
            UpdateKind::Message(message) => {
                // Print received text message to stdout.

                message_entry(&message, &api, &mut store, &connection).await?;
                // TODO: Remove debug codes
                // Extract text if the message is of kind Text
                if let MessageKind::Text { ref data, .. } = message.kind {
                    println!("<{}>: {}", &message.from.first_name, data);
                } else {
                    println!("<{}>: Non-text message", &message.from.first_name);
                }
            }
            UpdateKind::CallbackQuery(callback) => {
                callback_entry(&callback, &api, &mut store, &connection).await?;
            }
            _ => {
                // Unsupported message kind
                println!("Unsupported update kind received!");
            }
        }
    }
    Ok(())
}
