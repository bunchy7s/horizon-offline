//! The one account setting the game changes itself: "Show my cosmetics" (`PUT /v1/visible`).
//!
//! The game signs in the way an online-mode server checks a player: the service hands out a
//! one-time server id, the game tells Mojang's session server it joins it (`join`, with the
//! player's own Minecraft session), and the service asks Mojang. The Minecraft token goes to
//! Mojang only; the service's session lives for this one call and is never kept.
use serde_json::{Value, json};

/// A request to the service: `method` on `path`, with the service's session if any.
pub trait Call {
    fn call(
        &self,
        method: &str,
        path: &str,
        session: Option<&str>,
        body: &Value,
    ) -> Result<Value, String>;
}

impl Call for crate::Http {
    fn call(
        &self,
        method: &str,
        path: &str,
        session: Option<&str>,
        body: &Value,
    ) -> Result<Value, String> {
        let mut request = self
            .agent()
            .request(method, &format!("{}{path}", self.base()))
            .set("Content-Type", "application/json");
        if let Some(s) = session {
            request = request.set("Authorization", &format!("Bearer {s}"));
        }
        let bytes = crate::Http::read(request.send_bytes(body.to_string().as_bytes()))?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }
}

/// Turns "Show my cosmetics" on or off for the account `name`; `join` tells Mojang that account
/// joins the server id the service chose. Returns what the service now keeps.
pub fn set_visible(
    service: &impl Call,
    name: &str,
    join: impl FnOnce(&str) -> Result<(), String>,
    visible: bool,
) -> Result<bool, String> {
    let start = service.call("POST", "/v1/auth/start", None, &json!({}))?;
    let server_id = start["server_id"].as_str().ok_or("no server id")?;
    join(server_id)?;
    let finish = service.call(
        "POST",
        "/v1/auth/finish",
        None,
        &json!({ "server_id": server_id, "name": name }),
    )?;
    let session = finish["session"].as_str().ok_or("no session")?;
    let me = service.call(
        "PUT",
        "/v1/visible",
        Some(session),
        &json!({ "visible": visible }),
    )?;
    me["visible"].as_bool().ok_or_else(|| "no answer".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The service, as far as this call goes: what it was sent, and whether the id was joined.
    #[derive(Default)]
    struct Service {
        sent: RefCell<Vec<(String, String, Option<String>, Value)>>,
        joined: RefCell<Option<String>>,
    }

    impl Call for Service {
        fn call(
            &self,
            method: &str,
            path: &str,
            s: Option<&str>,
            body: &Value,
        ) -> Result<Value, String> {
            self.sent.borrow_mut().push((
                method.into(),
                path.into(),
                s.map(Into::into),
                body.clone(),
            ));
            match path {
                "/v1/auth/start" => Ok(json!({"server_id": "id-1"})),
                "/v1/auth/finish" if self.joined.borrow().as_deref() == Some("id-1") => {
                    Ok(json!({"session": "svc"}))
                }
                "/v1/auth/finish" => Err("HTTP 401".into()),
                "/v1/visible" if s == Some("svc") => Ok(json!({"visible": body["visible"]})),
                _ => Err("HTTP 401".into()),
            }
        }
    }

    #[test]
    fn hiding_goes_through_mojang_and_sends_only_the_services_session() {
        let service = Service::default();
        let shown = set_visible(
            &service,
            "Alex",
            |id| {
                *service.joined.borrow_mut() = Some(id.to_owned());
                Ok(())
            },
            false,
        );
        assert_eq!(shown, Ok(false));
        let sent = service.sent.borrow();
        let paths: Vec<&str> = sent.iter().map(|(_, p, ..)| p.as_str()).collect();
        assert_eq!(paths, ["/v1/auth/start", "/v1/auth/finish", "/v1/visible"]);
        assert_eq!(sent[1].3["name"], "Alex");
        // Only the last call carries a session, and it is the service's own.
        let sessions: Vec<Option<&str>> = sent.iter().map(|(.., s, _)| s.as_deref()).collect();
        assert_eq!(sessions, [None, None, Some("svc")]);
    }

    #[test]
    fn mojang_refusing_stops_before_the_service_is_asked_to_trust_it() {
        let service = Service::default();
        let refused = set_visible(&service, "Alex", |_| Err("403".into()), false);
        assert_eq!(refused, Err("403".into()));
        assert_eq!(service.sent.borrow().len(), 1);
        // A join Mojang never saw: the service refuses the id.
        assert!(set_visible(&service, "Alex", |_| Ok(()), false).is_err());
    }
}
