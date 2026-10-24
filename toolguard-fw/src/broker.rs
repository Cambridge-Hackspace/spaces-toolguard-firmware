//! The local broker, for a reader.
//!
//! ## Why this is not `spaces_device_espidf::run`
//!
//! That function does exactly this job and does it with the scars to prove it —
//! clean session, re-subscribe on every connect, `enqueue` rather than
//! `publish`, fragmented messages dropped loudly — but it also constructs a
//! `Module` and drives the power protocol's event loop, and its `Event` enum is
//! closed around `Swipe`/`ToolOff`/`Running`. A reader cannot borrow the
//! connection without the module.
//!
//! So the connection is rebuilt here and the four decisions are taken from
//! there deliberately rather than rediscovered:
//!
//! - **Clean session**, so the broker forgets our subscriptions when the link
//!   drops — which is why they are made again on every `Connected` rather than
//!   once at startup. A client that subscribes once is "connected and deaf"
//!   after the first reconnect, and `FIRMWARE.md` names that trap.
//! - **`enqueue`, never `publish`.** `publish` can block for up to ten seconds,
//!   and this is the same thread that has to keep polling the antenna. A member
//!   tapping a card while the broker is slow must not be ignored.
//! - **Fragmented messages are dropped, loudly.** Nothing on a reader's topics
//!   is anywhere near large enough to arrive in pieces, so a fragment is a
//!   surprise worth a log line rather than something to reassemble.
//! - **Errors are logged and are not events.** The reader's own health comes
//!   from `Connected`/`Disconnected`; a transient error between them is noise.
//!
//! The honest note: this is duplication, and the better fix is upstream — a
//! trait in `spaces-device` that both a `Module` and a `Reader` satisfy, so one
//! `run()` serves both. That is a change to a repository this one depends on by
//! tag, so it is not in this commit.

use std::sync::mpsc::Sender;
use std::time::Duration;

use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EventPayload, MqttClientConfiguration, QoS,
};
use spaces_device::wire::topic;
use spaces_device_espidf::store::key;
use spaces_device_espidf::Online;

/// What the broker tells the loop about.
pub enum Event {
    Connected,
    Disconnected,
    Message { topic: String, payload: Vec<u8> },
}

/// Connect, and forward every event worth acting on into `sender`.
///
/// The returned client is what publishes go through. Subscriptions are NOT made
/// here: they are made on each `Connected`, by the caller, for the reason above.
pub fn connect(online: &Online, sender: Sender<Event>) -> anyhow::Result<EspMqttClient<'static>> {
    let broker = online.store.get(key::BROKER).unwrap_or_default();
    let username = online.store.get(key::BROKER_USER).filter(|u| !u.is_empty());
    let password = online.store.get(key::BROKER_PASS).filter(|p| !p.is_empty());
    let client_id = online.credentials.device_id.clone();

    log::info!(
        "connecting to the local broker {broker} as {}",
        username.as_deref().unwrap_or("(no login)")
    );

    let client = EspMqttClient::new_cb(
        &broker,
        &MqttClientConfiguration {
            client_id: Some(&client_id),
            username: username.as_deref(),
            password: password.as_deref(),
            disable_clean_session: false,
            keep_alive_interval: Some(Duration::from_secs(10)),
            reconnect_timeout: Some(Duration::from_secs(3)),
            ..Default::default()
        },
        move |event| {
            let forwarded = match event.payload() {
                EventPayload::Connected(_) => Some(Event::Connected),
                EventPayload::Disconnected => Some(Event::Disconnected),
                EventPayload::Received {
                    topic: Some(topic),
                    data,
                    details: Details::Complete,
                    ..
                } => Some(Event::Message {
                    topic: topic.to_string(),
                    payload: data.to_vec(),
                }),
                EventPayload::Received { .. } => {
                    log::warn!("ignoring a message too large to arrive in one piece");
                    None
                }
                EventPayload::Error(e) => {
                    log::warn!("broker connection: {e:?}");
                    None
                }
                _ => None,
            };
            if let Some(event) = forwarded {
                let _ = sender.send(event);
            }
        },
    )?;

    Ok(client)
}

/// Subscribe to everything the edge might answer with.
///
/// Called on every `Connected`. The list is `spaces-device`'s, not a local one:
/// a reader only acts on `toolguard/response/tool-on`, but subscribing to the
/// same set the modules do means a topic added upstream reaches this firmware
/// too, where the core can ignore it explicitly rather than never see it.
pub fn resubscribe(client: &mut EspMqttClient<'static>) {
    for t in topic::SUBSCRIPTIONS {
        match client.subscribe(t, QoS::AtLeastOnce) {
            Ok(_) => log::info!("subscribed to {t}"),
            // Logged and not fatal: the next Connected will try again, and a
            // reader that cannot subscribe shows "No edge" rather than dying.
            Err(e) => log::warn!("could not subscribe to {t}: {e}"),
        }
    }
}
