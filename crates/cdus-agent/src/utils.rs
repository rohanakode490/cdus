use anyhow::Result;
use libp2p::PeerId;
use std::io::Read;

pub fn hex_to_peer_id(hex_pk: &str) -> Result<String> {
    let bytes = hex::decode(hex_pk)?;
    let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Invalid ed25519 public key: {}", e))?;
    Ok(PeerId::from_public_key(&libp2p::identity::PublicKey::from(pk)).to_string())
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&apos;", "'")
}

fn extract_title(html: &str) -> Option<String> {
    let html_lower = html.to_lowercase();
    if let Some(start_tag) = html_lower.find("<title>") {
        let content_start = start_tag + 7;
        if let Some(end_tag) = html_lower[content_start..].find("</title>") {
            let title = &html[content_start..content_start + end_tag];
            return Some(decode_html_entities(title.trim()));
        }
    }
    None
}

fn extract_favicon_url(html: &str, page_url: &str) -> String {
    let parsed_page_url = match url::Url::parse(page_url) {
        Ok(u) => u,
        Err(_) => return String::new(),
    };
    
    let mut favicon_href = None;
    let html_lower = html.to_lowercase();
    for link_start in html_lower.match_indices("<link") {
        let idx = link_start.0;
        if let Some(end_idx) = html_lower[idx..].find('>') {
            let tag = &html[idx..idx + end_idx];
            let tag_lower = tag.to_lowercase();
            if tag_lower.contains("rel=") && (tag_lower.contains("icon") || tag_lower.contains("shortcut")) {
                if let Some(href_start) = tag_lower.find("href=") {
                    let rest = &tag[href_start + 5..];
                    let quote_char = rest.chars().next().unwrap_or('"');
                    let href_val = if quote_char == '"' || quote_char == '\'' {
                        rest[1..].split(quote_char).next().unwrap_or("")
                    } else {
                        rest.split_whitespace().next().unwrap_or("")
                    };
                    if !href_val.is_empty() {
                        favicon_href = Some(href_val.to_string());
                        break;
                    }
                }
            }
        }
    }
    
    let href = favicon_href.unwrap_or_else(|| "/favicon.ico".to_string());
    match parsed_page_url.join(&href) {
        Ok(resolved) => resolved.to_string(),
        Err(_) => format!("{}/favicon.ico", page_url),
    }
}

fn fetch_favicon_as_base64(url_str: &str) -> Option<String> {
    if url_str.is_empty() {
        return None;
    }
    let response = ureq::get(url_str)
        .timeout(std::time::Duration::from_secs(3))
        .call()
        .ok()?;
        
    let content_type = response.header("content-type").unwrap_or("image/x-icon").to_string();
    let mut bytes = Vec::new();
    response.into_reader().read_to_end(&mut bytes).ok()?;
    
    if bytes.is_empty() {
        return None;
    }
    
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{};base64,{}", content_type, b64))
}

pub fn resolve_url_metadata(url_str: &str) -> Option<(String, String)> {
    let response = ureq::get(url_str)
        .timeout(std::time::Duration::from_secs(5))
        .call()
        .ok()?;
        
    let final_url = response.get_url().to_string();
    let html = response.into_string().ok()?;
    
    let title = extract_title(&html).unwrap_or_else(|| {
        url::Url::parse(url_str)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| "Link".to_string())
    });
    
    let favicon_url = extract_favicon_url(&html, &final_url);
    let favicon_base64 = fetch_favicon_as_base64(&favicon_url);
    
    Some((title, favicon_base64.unwrap_or_default()))
}
