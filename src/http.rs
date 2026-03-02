use std::time::Duration;

pub fn http_post(url: &str, body: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build();
    let mut req = agent.post(url).set("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.set("X-API-Key", key);
    }
    match req.send_string(body) {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            Ok((status, text))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Ok((code, text))
        }
        Err(e) => Err(format!("{e}")),
    }
}

pub fn http_get(url: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build();
    let mut req = agent.get(url);
    if let Some(key) = api_key {
        req = req.set("X-API-Key", key);
    }
    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            Ok((status, text))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Ok((code, text))
        }
        Err(e) => Err(format!("{e}")),
    }
}
