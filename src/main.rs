use dotenv::dotenv;
use futures::StreamExt;
use omikuji_bot::*;
use std::env;
use telegram_bot::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv().ok();

    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let api = Api::new(token);

    // Establish a connection to database server
    let connection = establish_connection();

    // Fetch new updates via long poll method
    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        // If the received update contains a new message...
        let update = update?;
        match update.kind {
            UpdateKind::Message(tg_message) => {
                // Extract text if the message is of kind Text
                if let MessageKind::Text { ref data, .. } = tg_message.kind {
                    // Print received text message to stdout.
                    println!("<{}>: {}", &tg_message.from.first_name, data);

                    message_entry(&tg_message, &api).await?;
                }
            }
            UpdateKind::CallbackQuery(callback) => {
                callback_entry(&callback, &api, &connection).await?;
            }
            _ => {
                // Unsupported message kind
                println!("Unsupported update kind received!");
            }
        }
    }
    Ok(())
}
