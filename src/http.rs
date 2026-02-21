pub fn http_post(url: &str, body: &str, api_key: Option<&str>) -> Result<(u16, String), String> {
    let mut req = ureq::post(url).set("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.set("X-API-Key", key);
    }
    let resp = req.send_string(body).map_err(|e| format!("{e}"))?;
    let status = resp.status();
    let body = resp.into_string().unwrap_or_default();
    Ok((status, body))
}
