use eyre::{bail, ensure, Result};
use futures::pin_mut;
use rxml::{AsyncEventReadExt, AsyncParser, ResolvedEvent};
use tokio::io::AsyncBufRead;

pub async fn get_email_from_request(xml: impl AsyncBufRead) -> Result<String> {
    pin_mut!(xml);
    let mut result = String::new();
    let mut parser = AsyncParser::new(xml);

    loop {
        match parser.read().await? {
            Some(ResolvedEvent::StartElement(_, (_, name), _)) if name == "EMailAddress" => {
                break;
            }
            Some(_) => {}
            None => bail!("no email found in request body"),
        }
    }
    loop {
        match parser.read().await? {
            Some(ResolvedEvent::StartElement(_, (_, name), _)) => {
                bail!("No nested xml node {} allowed inside EMailAddress", name);
            }
            Some(ResolvedEvent::EndElement(_)) => {
                ensure!(!result.is_empty(), "no email found in request body");
                return Ok(result);
            }
            Some(ResolvedEvent::Text(_, text)) => {
                result.push_str(&text);
            }
            Some(_) => {}
            None => bail!("EOF in the middle of EMailAddress"),
        }
    }
}
